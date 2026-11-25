//! `remedy-mcp`: the RemedyUI MCP server. See `docs/MCP.md`.

use std::path::PathBuf;
use std::process::ExitCode;

use cargo_remedy::mcp::{Config, Server, Workspace, select_groups};

const HELP: &str = "\
remedy-mcp: a Model Context Protocol server for building and operating RemedyUI apps

USAGE:
    remedy-mcp [options]

Serves one client over standard input and output (how MCP clients start it), or streamable
HTTP on loopback with --http.

OPTIONS:
    --tools <groups>        Only these tool groups, comma-separated (default: every group in
                            this build): docs, markup, effects, scaffold, preview, testing, live, devtools
    --workspace <dir>       A folder tools may read and write (repeatable; default: the current
                            directory). Paths outside these folders are refused.
    --docs-root <dir>       Read the docs from this RemedyUI checkout now, instead of the docs
                            embedded when remedy-mcp was built
    --remedy-root <dir>     The RemedyUI checkout whose budget scenes budget_check runs
    --http [port]           Serve streamable HTTP on 127.0.0.1 (port 0 or none picks one) at
                            /mcp. Clients send `Authorization: Bearer <token>`; the token is
                            REMEDY_MCP_TOKEN, or a fresh random one printed on standard error.
    --list-tools            Print the tools these options offer, and exit
    --help, --version
";

struct Options {
    tools: Option<String>,
    workspace: Vec<PathBuf>,
    docs_root: Option<PathBuf>,
    remedy_root: Option<PathBuf>,
    http: Option<u16>,
    list: bool,
}

fn parse(args: &[String]) -> Result<Option<Options>, String> {
    let mut o = Options {
        tools: None,
        workspace: Vec::new(),
        docs_root: None,
        remedy_root: None,
        http: None,
        list: false,
    };
    let mut i = 0;
    let value = |i: usize, flag: &str| -> Result<String, String> {
        args.get(i + 1).cloned().ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(a) = args.get(i) {
        match a.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("remedy-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--tools" => {
                o.tools = Some(value(i, a)?);
                i += 1;
            }
            "--workspace" => {
                o.workspace.push(PathBuf::from(value(i, a)?));
                i += 1;
            }
            "--docs-root" => {
                o.docs_root = Some(PathBuf::from(value(i, a)?));
                i += 1;
            }
            "--remedy-root" => {
                o.remedy_root = Some(PathBuf::from(value(i, a)?));
                i += 1;
            }
            "--http" => {
                let port = args.get(i + 1).and_then(|p| p.parse::<u16>().ok());
                if port.is_some() {
                    i += 1;
                }
                o.http = Some(port.unwrap_or(0));
            }
            "--list-tools" => o.list = true,
            other => return Err(format!("unknown option `{other}`\n\n{HELP}")),
        }
        i += 1;
    }
    Ok(Some(o))
}

fn run(o: Options) -> Result<(), String> {
    let groups = select_groups(o.tools.as_deref()).map_err(|e| e.0)?;
    let roots = if o.workspace.is_empty() {
        vec![std::env::current_dir().map_err(|e| format!("the current directory: {e}"))?]
    } else {
        o.workspace
    };
    let mut config = Config::new(Workspace::new(roots).map_err(|e| e.0)?);
    config.docs_root = o.docs_root;
    config.remedy_root = o.remedy_root;
    let server = Server::builder(config).groups(&groups).build();
    if o.list {
        for g in server.groups() {
            println!("{}: {}", g.name(), g.summary());
            for t in cargo_remedy::mcp::group(g).map(|x| x.tools()).unwrap_or_default() {
                println!("  {}", t.name);
            }
        }
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the async runtime: {e}"))?;
    runtime.block_on(async move {
        match o.http {
            Some(port) => serve_http(server, port).await,
            None => server.serve_stdio().await.map_err(|e| e.0),
        }
    })
}

#[cfg(feature = "mcp-http")]
async fn serve_http(server: Server, port: u16) -> Result<(), String> {
    let token = std::env::var("REMEDY_MCP_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(remedy_ui_core::token::random_token);
    let http = cargo_remedy::mcp::http::HttpServer::bind(port, token.clone()).await.map_err(|e| e.0)?;
    eprintln!("remedy-mcp: streamable HTTP at {}", http.url());
    if std::env::var_os("REMEDY_MCP_TOKEN").is_none() {
        eprintln!("remedy-mcp: token {token}");
    }
    http.serve(server).await.map_err(|e| e.0)
}

#[cfg(not(feature = "mcp-http"))]
async fn serve_http(_server: Server, _port: u16) -> Result<(), String> {
    Err("this remedy-mcp was built without the `http` feature".into())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).and_then(|o| o.map_or(Ok(()), run)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("remedy-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}
