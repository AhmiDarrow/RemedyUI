# remedy-mcp: RemedyUI for AI agents

`remedy-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server. With it, an AI agent (Claude Code or any other MCP client) can build apps with RemedyUI: search the docs and the widget catalog, check `.rui` markup, create an app, look at its screens as images, audit them, and run its tests. It can also operate a running RemedyUI app that the person has granted it access to.

The tools' logic lives in the `remedy-tools` crate. `cargo remedy` offers the same tools as commands ([CLI.md](CLI.md)).

## Install

From a RemedyUI checkout:

```sh
cargo install --path crates/remedy-mcp
```

This needs no `remedy-ui` facade, app or checkout. The docs come with it: `cargo xtask docs-index` writes an index of the guides, design docs, every engine crate's rustdoc and the widget catalog into `crates/remedy-tools/data`, the index is committed (`cargo xtask check` fails when it is out of date), and the tools embed it, so a build from crates.io has the docs too. To follow a checkout as it changes, add `--docs-root <checkout>`. `new_app` uses the app templates directly (the `remedy-templates` crate, shared with `cargo remedy new`).

### Smaller builds

Each tool group is a cargo feature. `full` is the default and includes `http`. A build with fewer groups compiles only the engine crates those groups need:

| Feature | Tools | What it compiles in |
|---|---|---|
| `docs` | `docs_search`, `docs_read`, `widget_catalog`; the guides as resources | the docs index only |
| `markup` | `rui_check`, `rui_compile`, `rui_elements` | `remedy-ui-rui` without its runtime |
| `scaffold` | `new_app` | `remedy-templates` (the app templates, shared with `cargo remedy new`) |
| `preview` | `render_preview`, `list_previews`, `a11y_audit` | nothing (it builds a harness against the app) |
| `testing` | `run_tests`, `budget_check` | nothing (it runs Cargo) |
| `live` | `app_connect`, `app_snapshot`, `app_act`, `app_command`, `app_undo`, `app_wait`, `app_disconnect` | nothing (it speaks the protocol) |
| `devtools` | `devtools_connect`, `devtools_tree`, `devtools_element`, `devtools_semantics`, `devtools_signals`, `devtools_updates`, `devtools_timings`, `devtools_disconnect` | nothing (it speaks the protocol) |
| `http` | streamable HTTP (`--http`) | hyper |

```sh
cargo install --path crates/remedy-mcp --no-default-features --features docs,markup
```

Even a full build can serve fewer tools: `--tools docs,live` offers only those groups. A name that isn't a group, or a group not built in, is refused at start.

## Set up in Claude Code

Add the server, with the folder the agent may work in as its workspace:

```sh
claude mcp add remedy -- remedy-mcp --workspace /path/to/your/apps
```

To share it with a project, put a `.mcp.json` in the project's root:

```json
{
  "mcpServers": {
    "remedy": {
      "command": "remedy-mcp",
      "args": ["--workspace", ".", "--tools", "docs,markup,scaffold,preview,testing,live"]
    }
  }
}
```

Other MCP clients start it the same way: the command `remedy-mcp`, talking over standard input and output.

### Streamable HTTP

For clients that connect to a running server instead of starting one:

```sh
REMEDY_MCP_TOKEN=some-long-secret remedy-mcp --http 7801 --workspace ~/apps
```

The server listens on `http://127.0.0.1:7801/mcp`. Every request needs `Authorization: Bearer some-long-secret`. Without `REMEDY_MCP_TOKEN`, a random token is printed on standard error. Each client session gets its own app and devtools connections.

## Options

| Option | Meaning |
|---|---|
| `--tools <groups>` | Only these groups, comma-separated (default: every group in the build) |
| `--workspace <dir>` | A folder tools may read and write. Repeat it for more. The default is the current directory |
| `--docs-root <dir>` | Read the docs from this RemedyUI checkout now, instead of the committed index |
| `--remedy-root <dir>` | The checkout whose budget scenes `budget_check` runs. The default is the one `remedy-mcp` was built from |
| `--http [port]` | Serve streamable HTTP on loopback instead of stdio |
| `--list-tools` | Print the tools these options offer, then exit |

## Tools

### Building with RemedyUI

- **`docs_search`** `{query, kind?, limit?}`: searches the guides, porting guides and recipes (`remedy_ui::guide`, in `crates/remedy-ui/src/guide`), the design docs (`docs/`) and every engine crate's rustdoc (one page per source file). Results are ranked with BM25, and headings and paths count for more than body text. Each hit has its path, line, heading and an excerpt.
- **`docs_read`** `{path, heading?, from_line?, lines?}`: reads a document whole, one section, or a range of lines. An empty path lists every document.
- **`widget_catalog`** `{name?, feature?}`: lists every widget builder of the `remedy_ui` facade. Each entry has:
  - its path (`remedy_ui::widgets::button`), its `remedy-ui` feature and its signature
  - its doc summary
  - the element names it creates, which the theme's style sheet keys its rules by, and which of those the Remedy sheet styles
  - its variants
  - an example. Deprecated builders are left out, since their replacements are listed (from its docs, or a call that shows each argument by its type)
  - its source line
- **`rui_check`** `{source | path, file?, elements?}`: checks `.rui` markup against the built-in elements, plus the app's own elements passed as specs (`{name, params: [{name, kind, required}], children, path}`). It reports the first problem at `file:line:column` with a hint. A valid file returns the signals it binds (with their Rust types) and the commands it names.
- **`rui_compile`**: the same input. Returns the Rust that `remedy_ui_rui::build` would generate. App elements need their builder's `path`.
- **`rui_elements`**: lists the built-in elements and their arguments.
- **`new_app`** `{name, folder?, template?, remedy_path?}`: creates an app from the app templates (the same as `cargo remedy new`) (`basic`, `tray`, `multi-window`, `document`, `android`) in a new folder inside the workspace, and lists the files it made.
- **`render_preview`** `{app, function, width?, height?, scale?, theme?, locale?, text_scale?, save_png?}`: renders part of an app headlessly. It returns:
  - a PNG (MCP image content)
  - the window as a screen reader meets it
  - the accessibility problems found
  
  More on this below.
- **`list_previews`** `{app}`: lists the previews the app declares with `#[preview]`. Each has its path for `render_preview`, whether it is public (only public functions can be rendered by path), its attribute (the themes, locales and sizes it asks for) and its source line.
- **`a11y_audit`**: the same input. Returns only the problems: unnamed controls, contrast below WCAG AA, targets under 24×24, pointer-only controls, focus that can't be seen, content past the window's edge, and controls a screen reader can't reach. Each problem names the source line that built the element.
- **`run_tests`** `{app, filter?, package?, release?, include_ignored?}`: runs `cargo test` and summarizes it. The summary has the counts, each test's outcome, each failure's output, and compiler errors with their locations.
- **`budget_check`** `{scene?, list_only?}`: runs the engine's budget scenes in release builds and reports which met their budget. Examples are the 50,000-file explorer and the 300-page zoom. Run it only when asked, because it builds the engine in release mode.

#### How previews work

An app exposes what to preview as plain public functions in its library, `pub fn home(ui: &Ui) -> El`. The templates' `home` is one, and so is every public `#[preview]` function (`list_previews` finds them). `function` names one by its path in the crate: `home`, `screens::settings`, `previews::dark_home`.

For each preview, the server does this:

1. It writes a small harness crate into `<app's target>/remedy-tools/preview/<crate>`. The harness depends on the app, and on the app's own `remedy-ui` dependency with the `devtools` feature added.
2. It builds the harness into the app's target folder, so the harness shares the app's compiled dependencies.
3. It runs the harness. The harness renders with the devtools' `Preview`, using the CPU renderer, so the pixels are the same as the app's golden images.

The first preview of an app compiles the engine and can take minutes. After that a preview takes seconds.

Preview options:

- `theme`: `dark`, `light`, or a Remedy palette id (`forest`, `neutral`, `deutan`, …).
- `locale`: any BCP 47 tag. Right-to-left locales (`ar-EG`, `he`) lay out mirrored. The pseudo-locales `en-XA` (accented and longer) and `ar-XB` (forced right to left) show layout problems before any translation exists.

If the code doesn't compile, the tool returns the compiler's errors with their locations. If the code panics, it returns the panic message.

### Operating a running app

An app opts in. It creates `remedy_ui::agents::Agents` for a window and calls `listen()`, and the person grants an agent access with a scope:

- `read`: see the window
- `act`: see and act
- `act_with_confirmation`: act, but the person confirms each action

The person gives the agent the listener's address and the grant's token.

- **`app_connect`** `{address, token, connection?}`: connects and returns the first snapshot. Several apps can be connected at once under different `connection` names.
- **`app_snapshot`**: the window's elements as a tree. Each element has its role, name, value, states, the actions it offers and its id. The snapshot also lists the app's commands (marking those that ask the person first) and what Undo would take back.
- **`app_act`** `{node, action, value?, wait_seconds?}`: acts on an element. The actions are `click`, `focus`, `increment`, `decrement`, `expand`, `collapse`, `scroll_into_view` and `set_value`.
- **`app_command`** `{id, wait_seconds?}`: runs one of the app's commands.
- **`app_undo`**: undoes the latest change in the window.
- **`app_wait`** `{wait_seconds?}`: waits for the person's answer to an earlier request.
- **`app_disconnect`**: closes the connection.

Some requests make the app ask the person first: commands the app marks destructive, and everything under a confirming grant. For these the app answers `asking`. The tool waits up to `wait_seconds` (60 by default) for the person's answer. If they haven't answered by then, it reports `asking`. Their answer then comes through `app_wait`, or is reported with the next call. Until it arrives, new actions and commands on that connection are refused.

### Devtools of a running app

A debug build serves its devtools with `Window::serve_devtools` or `RemoteServer`, which give the address and token.

- **`devtools_connect`** `{address, token}`: connects.
- **`devtools_tree`**: the element tree, with each element's box.
- **`devtools_element`** `{id}`: an element's details: its source line, style, states and semantics.
- **`devtools_semantics`**: the screen reader's view, with problems.
- **`devtools_signals`** `{id?}`: an element's bindings and what they read. With no id, the whole reactive graph.
- **`devtools_updates`**: why elements updated.
- **`devtools_timings`**: frame times (p50, p95, worst) and the last frame's phases.
- **`devtools_disconnect`**: closes the connection.

### Resources and prompts

- With the docs tools, every guide and design document is also a resource: `remedy-docs://docs/DESIGN.md`, and so on.
- The prompt **`build_screen`** `{screen, app?}` walks an agent through building a screen: docs, the widget catalog, writing it, previewing it in several themes and locales, fixing the audit, and testing it.
- The prompt **`operate_app`** `{task}` explains how to do a task in a running app, within the person's consent.

## Security model

- **Local only.** stdio needs no network. HTTP listens on `127.0.0.1` only, accepts only loopback `Host` headers (which guards against DNS rebinding), and answers only requests carrying the bearer token, compared in constant time.
- **The workspace.** Tools read and write only inside the `--workspace` folders, and resolve relative paths from the first one. Paths that lead outside a workspace folder, by `..` or by links, are refused. Building a preview writes only inside the app's target folder.
- **Consent for running apps.** The server can't reach an app the person hasn't opened to agents, and it connects only to loopback addresses. Every request carries the token of a grant the person made. The app checks each request on its own thread:
  - A read-only grant can't act.
  - Revoked or expired tokens are refused.
  - Secrets, locked content and the person's own prompts are out of reach.
  - Every change goes into the window's undo history under the agent's name.
  - Destructive commands wait for the person.
  
  Nothing in `remedy-mcp` works around this. It only speaks the app's protocol.
- **Devtools** are served by debug builds only (unless an app explicitly allows release), on loopback, behind their own token.

## Embedding

`remedy-mcp` is also a library. Other tools can serve some or all of the groups:

```rust
use remedy_mcp::{Config, Server};
use remedy_tools::{Group, Workspace};

let config = Config::new(Workspace::new([project_dir])?);
let server = Server::builder(config)
    .groups(&[Group::Docs, Group::Markup])
    .build();
server.serve_stdio().await?;
```

Each group is a `ToolGroup` (`remedy_mcp::group(Group::Docs)`). A tool of your own can be added with `ServerBuilder::group`. `Server::call(name, args)` calls a tool directly, without MCP. The logic under every tool is a plain function in `remedy-tools` (for example `remedy_tools::preview::render`), with typed input and output.

## Engine APIs the tools depend on

The tools use only these engine entry points, so engine changes reach them in one place:

- **The facade's `devtools` feature**, in the preview harness: `Preview`, `Variation`, `reader_lines`, `audit`, `is_rtl` and `Snapshot::to_png`. Also `remedy_ui::themes::PALETTES`, and `Ui::element(..).node_id()`.
- **`remedy_ui_rui`**: `Markup::parse`, `spec::{built_in, check, Kind, Param, Spec}` and `codegen::compile`.
- **The JSON-lines protocols** of the agents listener (`remedy-ui-automation`) and the remote devtools server (`remedy-ui-devtools`). The tests check these against the real crates.
- **The app templates** (`remedy-templates`), which follow the facade like any app.
