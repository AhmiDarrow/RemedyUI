//! `remedy-mcp`: a Model Context Protocol server that lets AI agents build apps with RemedyUI
//! and operate running RemedyUI apps.
//!
//! The tools' logic lives in [`crate::devkit`] (shared with `cargo remedy`'s commands); this
//! crate adapts it to MCP. Each tool group is a [`ToolGroup`], one per `cargo-remedy::devkit`
//! feature, and a [`Server`] serves the groups a [`ServerBuilder`] composed, so a build can
//! hold a few groups (cargo features) and a run can offer fewer still (`--tools docs,live`).
//!
//! ```no_run
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! use cargo_remedy::mcp::{Config, Server};
//! use cargo_remedy::devkit::{Group, Workspace};
//!
//! let config = Config::new(Workspace::new([std::env::current_dir()?])?);
//! let server = Server::builder(config).groups(&[Group::Docs, Group::Live]).build();
//! server.serve_stdio().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Security: tools only touch the workspace folders the person named; running apps are
//! reached on loopback with the address and token the person gave, under the consent the app
//! enforces; the HTTP transport listens on loopback only and needs a bearer token.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
#[cfg(any(feature = "mcp-live", feature = "mcp-devtools"))]
use std::sync::{Mutex, PoisonError};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, Role,
    ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use crate::devkit::{Group, ToolError, Workspace};

mod groups;
#[cfg(feature = "mcp-http")]
pub mod http;

/// What the tools may use: the workspace, and where to find RemedyUI's docs, its checkout
/// (for the docs and the budget scenes).
#[derive(Clone, Debug)]
pub struct Config {
    /// The folders tools may read and write.
    pub workspace: Workspace,
    /// A RemedyUI checkout to read the docs from now (instead of the committed index).
    pub docs_root: Option<PathBuf>,
    /// The RemedyUI checkout whose budget scenes `budget_check` runs (the one these tools
    /// were built from when absent).
    pub remedy_root: Option<PathBuf>,
}

impl Config {
    /// Tools working in `workspace`, with everything else found as usual.
    pub fn new(workspace: Workspace) -> Config {
        Config { workspace, docs_root: None, remedy_root: None }
    }
}

/// What a tool returns: text for the model, the same as JSON when it has structure, and
/// images.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    /// Readable text.
    pub text: String,
    /// The typed result as JSON.
    pub structured: Option<Value>,
    /// PNG images.
    pub png: Vec<Vec<u8>>,
}

impl Reply {
    /// A reply of `text` and `value` serialized (a value that is not an object is wrapped as
    /// `{"result": value}`: structured content is always an object).
    pub fn new(text: impl Into<String>, value: &impl serde::Serialize) -> Reply {
        let structured = serde_json::to_value(value).ok().map(|v| match v {
            Value::Object(_) => v,
            other => serde_json::json!({ "result": other }),
        });
        Reply { text: text.into(), structured, png: Vec::new() }
    }

    /// A reply of text alone.
    pub fn text(text: impl Into<String>) -> Reply {
        Reply { text: text.into(), structured: None, png: Vec::new() }
    }
}

/// A tool as offered: name, description for agents, input schema, and whether it only reads.
#[derive(Clone, Debug)]
pub struct ToolDef {
    /// Its name.
    pub name: &'static str,
    /// What it does and when to use it.
    pub description: &'static str,
    /// The JSON schema of its arguments.
    pub schema: serde_json::Map<String, Value>,
    /// It changes nothing.
    pub read_only: bool,
}

impl ToolDef {
    /// A tool taking `T` as its arguments.
    pub fn of<T: schemars::JsonSchema>(
        name: &'static str,
        description: &'static str,
        read_only: bool,
    ) -> ToolDef {
        let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default();
        let mut map = match schema {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        map.remove("$schema");
        map.remove("title");
        map.entry("type").or_insert_with(|| Value::String("object".into()));
        ToolDef { name, description, schema: map, read_only }
    }

    fn tool(&self) -> Tool {
        let mut annotations = ToolAnnotations::new();
        annotations.read_only_hint = Some(self.read_only);
        Tool::new(self.name, self.description, Arc::new(self.schema.clone())).annotate(annotations)
    }
}

/// The state tools share within one client's session: open connections to apps.
#[derive(Default)]
pub struct Session {
    #[cfg(feature = "mcp-live")]
    pub(crate) live: Mutex<crate::devkit::live::Live>,
    #[cfg(feature = "mcp-devtools")]
    pub(crate) devtools: Mutex<Option<crate::devkit::devtools::Devtools>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session")
    }
}

/// What a tool call can reach.
pub struct Context<'a> {
    /// The server's configuration.
    pub config: &'a Config,
    /// This client's connections.
    pub session: &'a Session,
    #[cfg_attr(not(feature = "mcp-docs"), allow(dead_code))]
    shared: &'a Shared,
}

impl Context<'_> {
    /// The docs library, loaded on first use.
    #[cfg(feature = "mcp-docs")]
    pub fn library(&self) -> Result<&crate::devkit::docs::Library, ToolError> {
        self.shared.library(self.config)
    }
}

/// A group of tools: its definitions and how to call them.
pub trait ToolGroup: Send + Sync {
    /// Which group it is.
    fn group(&self) -> Group;
    /// Its tools.
    fn tools(&self) -> Vec<ToolDef>;
    /// Calls tool `name` (one of [`ToolGroup::tools`]) with `args`. Blocking: the server runs
    /// it off its async threads.
    fn call(&self, cx: &Context<'_>, name: &str, args: Value) -> Result<Reply, ToolError>;
}

/// The tool group for `g`, when this build has it.
pub fn group(g: Group) -> Option<Box<dyn ToolGroup>> {
    groups::make(g)
}

/// Arguments as `T`, with a message naming what was wrong.
pub fn args<T: DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    let args = if args.is_null() { Value::Object(serde_json::Map::new()) } else { args };
    serde_json::from_value(args).map_err(|e| ToolError::new(format!("the arguments are not right: {e}")))
}

#[derive(Default)]
struct Shared {
    #[cfg(feature = "mcp-docs")]
    library: OnceLock<Result<crate::devkit::docs::Library, ToolError>>,
    #[cfg(not(feature = "mcp-docs"))]
    _none: OnceLock<()>,
}

impl Shared {
    #[cfg(feature = "mcp-docs")]
    fn library(&self, config: &Config) -> Result<&crate::devkit::docs::Library, ToolError> {
        self.library
            .get_or_init(|| match &config.docs_root {
                Some(root) => crate::devkit::docs::Library::from_checkout(root),
                None => crate::devkit::docs::Library::embedded(),
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Composes a [`Server`] from tool groups.
pub struct ServerBuilder {
    config: Config,
    groups: Vec<Box<dyn ToolGroup>>,
}

impl ServerBuilder {
    /// Adds a group (replacing one of the same kind).
    pub fn group(mut self, g: Box<dyn ToolGroup>) -> Self {
        let kind = g.group();
        self.groups.retain(|x| x.group() != kind);
        self.groups.push(g);
        self
    }

    /// Adds the built-in groups `gs` (those this build has).
    pub fn groups(mut self, gs: &[Group]) -> Self {
        for g in gs {
            if let Some(made) = group(*g) {
                self = self.group(made);
            }
        }
        self
    }

    /// Adds every group this build has.
    pub fn all_groups(self) -> Self {
        self.groups(&Group::built_groups())
    }

    /// The server.
    pub fn build(self) -> Server {
        let mut groups = self.groups;
        groups.sort_by_key(|g| g.group());
        Server {
            inner: Arc::new(Inner { config: self.config, groups, shared: Shared::default() }),
            session: Arc::new(Session::default()),
        }
    }
}

struct Inner {
    config: Config,
    groups: Vec<Box<dyn ToolGroup>>,
    shared: Shared,
}

/// The MCP server. Cloning shares the client session; [`Server::fresh_session`] gives a
/// server for another client.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
    session: Arc<Session>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.inner.groups.iter().map(|g| g.group().name()).collect();
        write!(f, "Server({})", names.join(", "))
    }
}

const INSTRUCTIONS: &str = "Tools for building apps with RemedyUI (a Rust UI engine) and operating running RemedyUI apps. \
To build: search the docs (docs_search, then docs_read) and the widget catalog before writing code; create apps with new_app; \
check markup with rui_check; look at your UI with render_preview (a PNG and the screen reader's view) and fix what a11y_audit finds; run_tests before you finish. \
To operate a running app: the person must grant access in the app and give you its address and token; use app_connect, then app_snapshot and app_act by node id. \
When an answer is 'asking', the person decides: wait for them (app_wait) and never try to get around it.";

impl Server {
    /// A builder with no groups yet.
    pub fn builder(config: Config) -> ServerBuilder {
        ServerBuilder { config, groups: Vec::new() }
    }

    /// The groups served.
    pub fn groups(&self) -> Vec<Group> {
        self.inner.groups.iter().map(|g| g.group()).collect()
    }

    /// Every tool, as offered.
    pub fn tool_defs(&self) -> Vec<ToolDef> {
        self.inner.groups.iter().flat_map(|g| g.tools()).collect()
    }

    /// The same tools with a session of its own (for another client).
    pub fn fresh_session(&self) -> Server {
        Server { inner: self.inner.clone(), session: Arc::new(Session::default()) }
    }

    /// Calls a tool directly (blocking), as an MCP client's call would.
    pub fn call(&self, name: &str, args: Value) -> Result<Reply, ToolError> {
        let cx = Context { config: &self.inner.config, session: &self.session, shared: &self.inner.shared };
        for g in &self.inner.groups {
            if g.tools().iter().any(|t| t.name == name) {
                return g.call(&cx, name, args);
            }
        }
        Err(ToolError::new(format!("no tool `{name}` here")))
    }

    fn has(&self, g: Group) -> bool {
        self.inner.groups.iter().any(|x| x.group() == g)
    }

    /// Serves one client over standard input and output until it closes.
    pub async fn serve_stdio(self) -> Result<(), ToolError> {
        let running = self
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| ToolError::new(format!("could not start serving: {e}")))?;
        running.waiting().await.map_err(|e| ToolError::new(format!("serving stopped: {e}")))?;
        Ok(())
    }

    #[cfg(feature = "mcp-docs")]
    fn guides(&self) -> Vec<(String, String)> {
        let cx = Context { config: &self.inner.config, session: &self.session, shared: &self.inner.shared };
        cx.library()
            .map(|l| {
                l.docs()
                    .iter()
                    .filter(|d| d.kind == crate::devkit::docs::DocKind::Guide)
                    .map(|d| (d.path.clone(), d.text.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(not(feature = "mcp-docs"))]
    fn guides(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// `bytes` as standard base64.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ABC.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

const RESOURCE_SCHEME: &str = "remedy-docs://";

fn prompts() -> Vec<(Group, Prompt)> {
    vec![
        (
            Group::Docs,
            Prompt::new(
                "build_screen",
                Some("Build a screen of a RemedyUI app: find the widgets, write it, preview it, audit it, test it."),
                Some(vec![
                    PromptArgument::new("screen").with_description("The screen to build, such as \"a settings screen with a theme picker and notification switches\"").with_required(true),
                    PromptArgument::new("app").with_description("The app's folder, if it exists already"),
                ]),
            ),
        ),
        (
            Group::Live,
            Prompt::new(
                "operate_app",
                Some("Do a task in a running RemedyUI app the person granted you access to."),
                Some(vec![
                    PromptArgument::new("task").with_description("What to do in the app").with_required(true),
                ]),
            ),
        ),
    ]
}

fn prompt_text(name: &str, args: &serde_json::Map<String, Value>) -> Option<String> {
    let arg = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match name {
        "build_screen" => {
            let app = arg("app");
            let where_ = if app.is_empty() {
                "Create the app first with new_app (template basic unless the screen calls for another)."
                    .to_owned()
            } else {
                format!("The app is in `{app}`.")
            };
            Some(format!(
                "Build {screen} with RemedyUI.\n\n{where_}\n\n\
                 1. Read how screens are built: docs_search for the topics involved (layout, the widgets you need, theming, localization), then docs_read the best hits.\n\
                 2. Pick widgets from widget_catalog (filter by name or feature); use their signatures exactly, and the `remedy-ui` features they need.\n\
                 3. Write the screen as `pub fn <name>(ui: &Ui) -> El` in the app's library, with every visible string in the app's i18n/en-US.ftl.\n\
                 4. render_preview it (dark and light, en-US and ar-EG for right to left, en-XA for long text) and look at the image and the screen reader's view.\n\
                 5. Fix every a11y_audit problem (names, contrast, target sizes, keyboard access).\n\
                 6. run_tests, and add a test for the screen's behaviour if it has any.",
                screen = arg("screen")
            ))
        }
        "operate_app" => Some(format!(
            "Do this in the running app: {task}\n\n\
             The person must have granted you access in the app and given you its address and token; ask them for both if you do not have them, and connect with app_connect. \
             Take app_snapshot to see the window: act on elements by their id with app_act, or run the app's commands with app_command. \
             Everything you do goes into the window's undo history under your name. When an answer is 'asking', the app is asking the person: wait for them with app_wait and respect a 'declined'. \
             Never try to reach what is out of your reach (secrets, locked content, the person's own prompts). Disconnect with app_disconnect when done.",
            task = arg("task")
        )),
        _ => None,
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        // Guides are resources with the docs tools; prompts come with docs or live tools.
        let caps = if self.has(Group::Docs) {
            ServerCapabilities::builder().enable_tools().enable_resources().enable_prompts().build()
        } else if self.has(Group::Live) {
            ServerCapabilities::builder().enable_tools().enable_prompts().build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
        ServerConfig::new(caps)
            .with_server_info(Implementation::new("remedy-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tool_defs().iter().map(ToolDef::tool).collect()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        if !self.tool_defs().iter().any(|t| t.name == name) {
            return Err(ErrorData::invalid_params(format!("no tool `{name}`"), None));
        }
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let me = self.clone();
        let outcome = tokio::task::spawn_blocking(move || me.call(&name, args))
            .await
            .map_err(|e| ErrorData::internal_error(format!("the tool stopped: {e}"), None))?;
        let result = match outcome {
            Ok(reply) => {
                let mut content = vec![ContentBlock::text(reply.text)];
                for png in &reply.png {
                    content.push(ContentBlock::image(base64(png), "image/png"));
                }
                let mut r = CallToolResult::success(content);
                r.structured_content = reply.structured;
                r
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.0)]),
        };
        Ok(CallToolResponse::Complete(result))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let me = self.clone();
        let guides = tokio::task::spawn_blocking(move || me.guides()).await.unwrap_or_default();
        Ok(ListResourcesResult::with_all_items(
            guides
                .iter()
                .map(|(path, text)| {
                    let title = text
                        .lines()
                        .find(|l| l.starts_with('#'))
                        .map(|l| l.trim_start_matches('#').trim().to_owned());
                    let mut r = Resource::new(format!("{RESOURCE_SCHEME}{path}"), path.clone())
                        .with_mime_type("text/markdown");
                    if let Some(t) = title {
                        r = r.with_title(t);
                    }
                    r
                })
                .collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let Some(path) = request.uri.strip_prefix(RESOURCE_SCHEME).map(str::to_owned) else {
            return Err(ErrorData::resource_not_found(format!("no resource {}", request.uri), None));
        };
        let me = self.clone();
        let guides = tokio::task::spawn_blocking(move || me.guides()).await.unwrap_or_default();
        let Some((_, text)) = guides.into_iter().find(|(p, _)| *p == path) else {
            return Err(ErrorData::resource_not_found(format!("no resource {}", request.uri), None));
        };
        let contents = match ResourceContents::text(text, request.uri.clone()) {
            ResourceContents::TextResourceContents { uri, text, meta, .. } => {
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some("text/markdown".into()),
                    text,
                    meta,
                }
            }
            other => other,
        };
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(vec![contents])))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(
            prompts().into_iter().filter(|(g, _)| self.has(*g)).map(|(_, p)| p).collect(),
        ))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let offered = prompts().into_iter().any(|(g, p)| p.name == request.name && self.has(g));
        let args = request.arguments.unwrap_or_default();
        match prompt_text(&request.name, &args).filter(|_| offered) {
            Some(text) => {
                Ok(GetPromptResponse::Complete(GetPromptResult::new(vec![PromptMessage::new_text(
                    Role::User,
                    text,
                )])))
            }
            None => Err(ErrorData::invalid_params(format!("no prompt `{}`", request.name), None)),
        }
    }
}

/// The groups for a `--tools` value, or every built group for `None`.
pub fn select_groups(list: Option<&str>) -> Result<Vec<Group>, ToolError> {
    match list {
        Some(l) => {
            let gs = Group::parse_list(l)?;
            if gs.is_empty() {
                return Err(ToolError::new("--tools names no groups"));
            }
            Ok(gs)
        }
        None => Ok(Group::built_groups()),
    }
}

/// Locks a mutex, carrying on past a panic in another holder.
#[cfg(any(feature = "mcp-live", feature = "mcp-devtools"))]
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn groups_compose_at_run_time() {
        let config = Config::new(Workspace::anywhere());
        let all = Server::builder(config.clone()).all_groups().build();
        assert_eq!(all.groups(), Group::built_groups());
        if Group::Docs.built() && Group::Live.built() {
            let some = Server::builder(config).groups(&[Group::Live, Group::Docs]).build();
            assert_eq!(some.groups(), vec![Group::Docs, Group::Live]);
            let names: Vec<&str> = some.tool_defs().iter().map(|t| t.name).collect();
            assert!(names.contains(&"docs_search") && names.contains(&"app_connect"));
            assert!(!names.contains(&"rui_check"));
            assert!(some.call("rui_check", Value::Null).is_err(), "a group left out is not callable");
        }
        assert!(select_groups(Some("docs,nope")).is_err());
        assert!(select_groups(Some(" , ")).is_err());
    }

    #[test]
    fn every_tool_has_an_object_schema_and_a_description() {
        let all = Server::builder(Config::new(Workspace::anywhere())).all_groups().build();
        let mut names = std::collections::HashSet::new();
        for t in all.tool_defs() {
            assert!(names.insert(t.name), "{} is defined once", t.name);
            assert_eq!(t.schema.get("type").and_then(Value::as_str), Some("object"), "{}", t.name);
            assert!(t.description.len() > 40, "{} explains itself", t.name);
        }
    }

    #[test]
    fn prompts_fill_in_their_arguments() {
        let mut args = serde_json::Map::new();
        args.insert("screen".into(), Value::String("a settings screen".into()));
        assert!(prompt_text("build_screen", &args).is_some_and(|t| t.contains("Build a settings screen")));
        assert!(prompt_text("nope", &args).is_none());
    }
}
