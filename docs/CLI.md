# cargo remedy

`cargo remedy` is RemedyUI's command line. It creates, runs and packages apps. It also offers every developer tool that `remedy-mcp` gives AI agents ([MCP.md](MCP.md)): one logic, `cargo-remedy::devkit`, behind both.

```sh
cargo install --path crates/cargo-remedy
```

Every tool command prints readable text. Add `--json` to get the typed result as JSON for scripts. A command exits with failure when what it checks fails: a markup error, an accessibility problem, a failing test, a missed budget, or a refused request.

## Apps

| Command | What it does |
|---|---|
| `cargo remedy new <name> [--template basic\|tray\|multi-window\|document\|android] [--path <dir>] [--remedy-path <checkout>]` | Creates an app: a window, the Remedy theme, Fluent strings, a test, a golden-image preview and `remedy.toml` |
| `cargo remedy run [--release] [--watch] [-- <args>]` | Builds and runs it. In debug builds, F12 opens the inspector. `--watch` rebuilds and restarts the app on change |
| `cargo remedy package [--target windows\|linux\|macos\|android\|web]` | Builds a release and packages it (installer, portable zip, `.tar.gz`, `.app`, APK; signatures; for `web`, an installable page, with `--example <name>`, `--dev` and `--out <dir>`) |
| `cargo remedy migrate [--dry-run] [--path <dir>]` | Rewrites an app for the engine's renamed APIs (see `CHANGELOG.md`) and for the engine crates that became modules of others (`remedy_ui_viz::bars` becomes `remedy_ui_widgets::viz::bars`) |

## Docs and widgets

| Command | What it does |
|---|---|
| `cargo remedy docs <query> [--kind guide\|rustdoc] [--limit <n>]` | Searches the guides, design docs and every engine crate's rustdoc. Each hit has its path, line, heading and an excerpt |
| `cargo remedy docs read <path> [--heading <h>] [--from <line>] [--lines <n>]` | Reads a document, one section of it, or a range of its lines |
| `cargo remedy docs list` | Lists every document |
| `cargo remedy widgets [name] [--feature <feature>]` | The widget catalog: path, feature, signature, docs, the element names the style sheet keys its rules by, variants, an example and the source line |

The docs come from the index committed with the tools (`cargo xtask docs-index` writes it; `cargo xtask check` fails when it is out of date). `--docs-root <checkout>` reads a checkout as it is now instead.

## Markup

| Command | What it does |
|---|---|
| `cargo remedy rui check <file.rui> [--elements <specs.json>]` | Checks the file. It prints the signals and commands the file binds, or fails with the first error at `file:line:column` |
| `cargo remedy rui compile <file.rui> [--elements <specs.json>] [--out <file.rs>]` | Prints (or writes) the Rust the build script would generate |
| `cargo remedy rui elements` | Lists the built-in elements and their arguments |
| `cargo remedy rfx check <file.rfx or built-in name> [--untrusted]` | Checks a Prism effect against an app's limits (or a stranger's with `--untrusted`). It prints the parameters and each pass's cost, or fails with every problem drawn under its line, with a suggestion for misspelled names |
| `cargo remedy rfx render <file.rfx or name> --png <file> [--size <w>x<h>] [--scale <s>] [--time <t>] [--set name=v[,v..]]... [--background #rrggbb\|none]` | Renders a frame of the effect on the CPU to a PNG the size of the element (`--size` times `--scale`), stretched smoothly as the element draws it, and says the resolution the effect drew at |
| `cargo remedy rfx library` | Lists the built-in effects with what each is and costs |

`--elements` names a JSON file listing the app's own elements, such as:

```json
[{"name": "fretboard", "params": [{"name": "strings", "kind": "count"}], "children": false, "path": "crate::ui::fretboard"}]
```

Each param `kind` is one of `label`, `text`, `flag`, `number`, `choice`, `items`, `level`, `command`, `range`, `count`, `space`, `switch` or `word`. `path` is needed only to compile.

## Previews and accessibility

| Command | What it does |
|---|---|
| `cargo remedy preview <function> [--png <file>] [--size <w>x<h>] [--scale <s>] [--theme dark\|light\|<palette>] [--locale <tag>] [--text-scale <s>] [--app <dir>]` | Renders `fn(&Ui) -> El` from the app's library (such as `home` or `screens::settings`) headlessly. It writes the PNG, then prints the screen reader's view and the accessibility problems |
| `cargo remedy previews [--app <dir>]` | Lists the app's `#[preview]` functions, with their attributes and whether they can be rendered by path (public) |
| `cargo remedy audit <function> [same options]` | Prints only the accessibility problems, and fails if there are any |

`--app` defaults to the current directory. The first preview builds a small harness against the app's dependencies in its target folder, which can take minutes. Later previews take seconds. Previews use the CPU renderer, so they draw the same pixels as golden images. [MCP.md](MCP.md#how-previews-work) explains how this works.

To check the reflowing and mirroring of a screen:

```sh
cargo remedy preview home --locale ar-EG --png home-rtl.png
cargo remedy preview home --locale en-XA --text-scale 1.5 --png home-long.png
cargo remedy audit home --theme light
```

## Tests and budgets

| Command | What it does |
|---|---|
| `cargo remedy test [filter] [--release] [--include-ignored] [-p <package>] [--app <dir>]` | Runs `cargo test` and summarizes it: counts, each failure's output, and compiler errors with their locations |
| `cargo remedy budgets [scene] [--list] [--remedy-root <checkout>]` | Runs the engine's budget scenes in release builds (or lists them) and reports which met their budget |

## Running apps

An app opens its agents listener (`remedy_ui::agents::Agents::listen`), and the person grants access with a scope:

- `read`: see the window
- `act`: see and act
- `act_with_confirmation`: act, but the person confirms each action

The person gives you the listener's address and the grant's token. Pass them as `--address` and `--token`, or set `REMEDY_APP_ADDRESS` and `REMEDY_APP_TOKEN`.

| Command | What it does |
|---|---|
| `cargo remedy app connect` | Checks the token, then shows what the window offers |
| `cargo remedy app snapshot` | Shows the window's elements (role, name, value, states, actions and id), the app's commands, and what Undo would take back |
| `cargo remedy app act <node> <action> [value] [--wait <s>]` | Acts on an element: `click`, `focus`, `increment`, `decrement`, `expand`, `collapse`, `scroll_into_view`, or `set_value <text>` |
| `cargo remedy app command <id> [--wait <s>]` | Runs one of the app's commands |
| `cargo remedy app undo` | Undoes the latest change in the window |

Each command connects, does one thing and disconnects. Some requests make the app ask the person first: destructive commands, and everything under a confirming grant. The command then waits for the person's answer, 60 seconds by default (`--wait`). If the person declines, the command fails. Every change goes into the window's undo history under the grant's name.

## Devtools of a running app

Debug builds serve their devtools on loopback with a token (`Window::serve_devtools`). Pass the address and token as `--address` and `--token`, or set `REMEDY_DEVTOOLS_ADDRESS` and `REMEDY_DEVTOOLS_TOKEN`.

| Command | What it does |
|---|---|
| `cargo remedy devtools tree` | The element tree, with each element's box |
| `cargo remedy devtools element <id>` | An element's details: its source line, style, states and semantics |
| `cargo remedy devtools semantics` | The screen reader's view and the accessibility problems |
| `cargo remedy devtools signals [id]` | An element's bindings and what they read. With no id, the whole reactive graph |
| `cargo remedy devtools updates` | Why elements updated recently |
| `cargo remedy devtools timings` | Frame times (p50, p95, worst) and the last frame's phases |
