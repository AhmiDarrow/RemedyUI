# API review for 1.0

The review before the 1.0 freeze (ROADMAP M10): every public item of every crate, checked
for consistent names, nothing public by accident, room to grow, typed errors and documented
entry points, with each gap fixed at its root. The rules it applied are in
[STABILITY.md](STABILITY.md); the release tools that keep them are in
[RELEASING.md](RELEASING.md).

**Method.** An inventory of the public items (about 3,300: 2,450 functions, 440 structs, 180
enums, 150 constants, 22 traits, 19 aliases), searched for each rule below; a struct-literal
census across the workspace and FreeSpeak (the one Remedy app on the engine whose sources
are here) to size each change; then each crate's surface read where a rule flagged it.
`remedy-ui-engine::script` and `remedy-ui-widgets::board` were rebuilt on a language-neutral
interface with WebAssembly, and later on RemedyLua in place of the Rhai placeholder
("Scripted widgets" below).

**For other branches merging after this one:** every rename below is in
`crates/cargo-remedy/migrations.toml`, so `cargo remedy migrate --path <crate>` applies the
mechanical ones to a branch's own code; the struct-literal and `match` changes are listed
under "Room to grow" with the replacement for each.

## Renames

| Crate | Old | New | Why |
|---|---|---|---|
| `remedy-ui-text` | `StylePatch` | `TextStylePatch` | Two public types were named `StylePatch` (element styles in `remedy-ui-core::style`, span styles here), both reachable through the facade. One name per concept. |
| `remedy-ui-engine::view` | re-export `TextPatch` | `TextStylePatch` | The same type under a third name. |
| `remedy-ui-platform::bridge_headless` | `HeadlessWindow::{maximized, fullscreen, minimized, frame_requested}` | `is_maximized`, `is_fullscreen`, `is_minimized`, `is_frame_requested` | Boolean states read `is_*` everywhere, as on the facade's `Window` (`is_fullscreen`, `is_maximized`, `is_visible`). |
| `remedy-ui-platform::bridge_headless` | `HeadlessHost::exited` | `has_exited` | Same rule, for an event that happened. |
| `remedy-ui-media-playback` | `Player::{muted, fullscreen}` | `is_muted`, `is_fullscreen` | Pairs `set_muted` / `request_fullscreen` with the facade's `set_*` / `is_*`. |
| `remedy-ui-platform::embed_webview` | `WebView::loading` | `is_loading` | Boolean state. |
| `remedy-ui-engine::view` | `Ui::occluded` | `is_occluded` | Boolean state. |
| `remedy-ui-widgets-term` | `Terminal::monospace` | `is_monospace` | Boolean state. |
| `remedy-ui-engine::view` | `Ui::set_time(ms: u64)` | `Ui::set_clock(Duration)` | The unit was only in a parameter name; the setter now takes a `Duration`. Not mechanical (the argument changes type), so not in the migration table. |
| `remedy-ui-widgets::base` | `label(ui, s, variant)` (deprecated, kept) | `text(ui, s).variant(variant)` | The variant was always required; `text` plus the `variant` builder is the one way, with the variant optional. FreeSpeak uses `label`, so it stays deprecated for a release. `cargo remedy migrate` rewrites the calls. |
| `remedy-ui-widgets::rui` | `build::compile_dir_to(dir, out, customs, cargo)` | `compile_dir_to(root, dir, out, customs, cargo)` | Compiled markup names the runtime by a root path (below). |

Names examined and kept, with the rule they follow:

- `*Options` for what an app passes to build or adjust one thing (24 types); `*Config` for the
  two process-level setups that say what an app or its updater is (`AppConfig`,
  `UpdateConfig`); `*Settings` for values that come from the platform or the person
  (`SystemSettings`, `GestureSettings`, `AndroidSettings`).
- `on_*` for callbacks (40), in the present tense for repeating events (`on_click`,
  `on_change`, `on_resize`) and the past participle for once-after ones (`Window::on_closed`,
  `El::on_removed`); `observe_text` / `observe_caret` are kept because, unlike `on_change`,
  any number of them can be added.
- `label` is text a person reads on a control, `title` a heading of a window, section, column
  or board, `name` an identifier or proper name; checked across every public field.
- Properties take `impl IntoProp<T>` (a value, a `Signal`, a `Memo` or a closure), so a
  reactive property has the same name and shape as a constant one: `El::text`, `disabled`,
  `variant`, `semantics`, `style`, and the core widgets' labels and texts (`button`,
  `nav_item`, `text`, `heading`, `link`, `check`, `switch`, …). The `*_dyn` doubles this made
  redundant (`El::text_dyn`, `button_dyn`, `nav_item_dyn`) are deprecated for a release, and
  `style_dyn`, `disabled_dyn` and `semantics_dyn`, which no app used, are gone; `cargo remedy
  migrate` rewrites all of them. `*_dyn` stays only where the whole thing is rebuilt from a
  closure rather than one property bound (`App::tray_dyn`); `rich::markdown` takes
  `IntoProp` like everything else (below).
- Typed design tokens are named by role, not by the types they resolve to: `Space`, `Radius`,
  `TextRole`, `ColorRole` (`TextStyle` and `Color` already name the text engine's style and the
  color value), with `Dp`, `ColorRef`, `Pad` and the token `Style` patch carrying either.
- Clocks: `Ui::set_clock` is the edit clock undo groups by; `Ui::set_frame_time` /
  `frame_time` is the frame clock animations and painters (`PaintCx::time`) sample.
- Units: durations are `Duration`; the nanosecond fields of trace and devtools types end in
  `_ns`; no other numeric time is public.

## Errors

Every fallible public function, trait method and callback now returns a typed error that
implements `std::error::Error` and `Display`. No `Result<_, String>` is public.

| Crate | Old | New |
|---|---|---|
| `remedy-ui` | `RunError(pub String)` | `#[non_exhaustive] enum RunError { SingleInstance, Bridge, UnsupportedPlatform, NeedsFeature }` |
| `remedy-ui` | `App::open_url(.., FnOnce(Result<(), String>))` | `FnOnce(Result<(), host::Unsupported>)` |
| `remedy-ui-platform::bridge_winit`, `remedy-ui-bridge-web` | `RunError(pub String)` | opaque `RunError` (`new`, `reason()`) |
| `remedy-ui-core` | `exec::SpawnError(pub String)` | opaque (`reason()`) |
| `remedy-ui-widgets::devtools` | `Snapshot::to_png / from_png -> Result<_, String>` | `PngError`; `GoldenError` implements `Error` and gains `Png` |
| `remedy-ui-platform::services_desktop` | `autostart::set`, `set_in`, `reveal -> Result<(), String>` | `DesktopError` (`NotFound`, `NoAutostartFolder`, `Io`, `Platform`) |
| `remedy-ui-widgets::spell` | `SpellBackend::{check_word, suggest}`, `BackendFactory` with `String` | `BackendError` (`From<String>` / `From<&str>`, so back ends keep using `?`); `SpellError::Backend(BackendError)` with `source()` |
| `remedy-ui-platform::embed_gpu` | `GpuContent::prepare -> Result<(), String>` | `PrepareError` |
| `remedy-ui-widgets::base` | validators returning `Result<(), String>` | `Rejection`, the reason an app turns down a person's edit, used by `Column::editable`, `TreeState::set_import`, `MoveBox` and `Form::field` |
| `remedy-ui-widgets::nav` | (new) | `BindingsError` for imported shortcut bindings |
| `remedy-ui-core` | (new) | `ParseEditionError` |

## Room to grow

`#[non_exhaustive]` now marks 139 enums and 126 structs; 40 enums stay exhaustive with a
comment saying why. Per crate:

| Crate | Changed |
|---|---|
| `remedy-ui` | enums: RendererChoice, RendererKind, RunError; structs: AppConfig, WindowSpec |
| `remedy-ui-core::a11y` | enums: Action, Live, Role; structs: ReaderNode |
| `remedy-ui-widgets::automation` | enums: AgentAction, Answer, Refused, Request, Scope; structs: CommandNode, Node, Snapshot |
| `remedy-ui-platform::bridge_android` | structs: AndroidSettings |
| `remedy-ui-bridge-web` | enums: Mount; structs: RunError, WebConfig |
| `remedy-ui-platform::bridge_winit` | enums: Backend, UserEvent, WindowNotice; structs: RunError |
| `remedy-ui-engine::compositor` | structs: FrameReport; kept exhaustive: CompositeOp |
| `remedy-ui-core` | enums: Edition, ParseColorError |
| `remedy-ui-widgets::customize` | enums: Area, CustomizeError, RefusalKind, TokenKind, TokenValue; structs: ContrastWarning, ImportPolicy, ImportReport, Notice, Refusal |
| `remedy-ui-widgets::devtools` | enums: GoldenError, PngError, ProblemKind, RemoteAnswer, RemoteRequest, Unreachable; structs: FrameSample, Problem, ReaderLine, RemoteBinding, RemoteElement, RemoteFrame, RemoteNode, RemoteOptions, RemoteProblem, RemoteRead, RemoteReaderNode, RemoteUpdate, Slice, UpdateEntry |
| `remedy-ui-widgets::doc_code` | enums: DiffLine, Kind; structs: CodeOptions |
| `remedy-ui-widgets::doc_pages` | enums: AnnotationKind, Fit, RenderError, Shape, Spread, Tool; structs: PageStats, RenderRequest, SearchStatus; kept exhaustive: Rotation |
| `remedy-ui-widgets::doc_rich` | structs: RichEditorOptions |
| `remedy-ui-platform::embed_gpu` | enums: EmbedInput, EmbedStatus; structs: EmbedStats, Gpu, PrepareError, Target |
| `remedy-ui-platform::embed_webview` | enums: BlockReason, NavigationSource, WebViewStatus; structs: Blocked, DownloadRequest, NavigationRequest |
| `remedy-ui-fonts` | enums: Style |
| `remedy-ui-core::host` | enums: TrayEvent; structs: FileDialog, SystemSettings, TextInputState, TrayIcon, Unsupported, WindowOptions; kept exhaustive: AccessibilityEvent, DialogStarted, DragEvent, DragStarted, DropEffect, HostEvent, ImeEvent, MemoryPressure, ResizeEdge, TrayItem (after 1.0.0-rc.1 it gained `Check`, `Submenu` and `Item::default`: a major change by design, listed in the CHANGELOG under Changed) |
| `remedy-ui-core::input` | enums: Claim, GestureKind, Key, Platform, PointerKind, Scope; structs: ChordError, Clash, Conflict, GestureSettings, KeyEvent, PinchEvent, PointerEvent, ScrollPhysics, WheelEvent; kept exhaustive: Modifiers, PinchPhase, PointerPhase |
| `remedy-ui-core::layout` | enums: Align, Display, JumpAlign, Justify, Length, Position, Track; structs: LayoutStyle, MeasureInput; kept exhaustive: Available, Axis, Direction |
| `remedy-ui-media::media_anim` | kept exhaustive: Loops, Opened |
| `remedy-ui-media::media_image` | enums: DecodeError, Fit, ImageState, Source; structs: Decoded |
| `remedy-ui-media::media_isolated` | enums: EnableError, Layer; structs: Isolation, Stats |
| `remedy-ui-media-playback` | enums: MediaError, MediaSource, PlaybackState, Sample, SubtitleFormat, UnseenPolicy; structs: AudioFormat, MediaInfo, PlaybackStats, PlayerOptions, TextTrackInfo, VideoInfo |
| `remedy-ui-media::media_video` | enums: VideoFit; structs: VideoStats |
| `remedy-ui-core::motion` | enums: Easing, Motion |
| `remedy-ui-engine::paint` | kept exhaustive: Blend, Brush, Item, PathEl, PixelFormat, Sampling, StrokeCap, StrokeJoin, YuvMatrix |
| `remedy-ui-core::reactive` | enums: NodeKind, ReactiveError, Segment, StoreError, VecDiff; structs: Cause, NodeInfo, PostReport, Stats, Update |
| `remedy-ui-engine::render_cpu` | enums: ImageKey, RenderError |
| `remedy-ui-engine::render_gpu` | enums: GpuError, Presented, Rung; structs: LadderOptions |
| `remedy-ui-widgets::rui` | enums: Binding, Bound, BuildError, Kind, SaveError, Space, Value; structs: RuiError |
| `remedy-ui-platform::services_desktop` | enums: DesktopError, KnownFolder |
| `remedy-ui-platform::services_i18n` | enums: Arg, DecimalError, Problem; structs: OverrideProblem; kept exhaustive: Weekday |
| `remedy-ui-platform::services_net` | enums: NetError; structs: HttpError, Response |
| `remedy-ui-widgets::spell` | enums: Availability, Prepared, SpellError; structs: BackendError, BackendReport, MenuLabels, SpellOptions |
| `remedy-ui-widgets::update` | enums: Apply, Feed, SignatureStatus, Trust, UpdateError, UpdateState; structs: BannerText, Release, UpdateConfig |
| `remedy-ui-core::style` | enums: Density; structs: ColorRoles, DerivedTokens, MotionTokens, Radii, Spacing, StylePatch, Theme, ThemeAssets, TypeScale |
| `remedy-ui-text` | enums: Align, Antialias, Hinting, LineHeight, Pseudo; structs: AtlasStats, GlyphRunInfo, LineInfo, ParagraphOptions, TextStyle, TextStylePatch; kept exhaustive: Affinity, Block, Direction, Mark |
| `remedy-ui-widgets::themes` | enums: ThemeChoice, TokenError |
| `remedy-ui-core::tree` | enums: HitMode, Phase, TreeError; structs: Dispatch, Failure |
| `remedy-ui-engine::view` | enums: DecorationKind, FieldKind, TextInput; structs: DirtyRecord, ElementInfo, FrameStats, HistoryItem, Placement; kept exhaustive: Request |
| `remedy-ui-widgets::viz` | enums: Range, Zone; structs: BarsOptions, MeterOptions, RingOptions, SparklineOptions; kept exhaustive: Orientation |
| `remedy-ui-widgets::base` | enums: Icon; structs: Rejection |
| `remedy-ui-widgets::data` | enums: Change, SortValue; kept exhaustive: SortDirection |
| `remedy-ui-widgets::extra` | enums: QrError; structs: CarouselOptions, EncodeOptions, QrOptions, RatingOptions, StepperLabels, SwatchOptions; kept exhaustive: Ecc, Mode |
| `remedy-ui-widgets::forms` | enums: SubmitState; structs: NumberOptions |
| `remedy-ui-widgets::music` | enums: Clef, Inlays, MarkKind, NoteValue, StringState, Technique; structs: ChordOptions, FretboardOptions, NoteStaffOptions, TabOptions; kept exhaustive: Step |
| `remedy-ui-widgets::nav` | enums: Edge; structs: CommandInfo |
| `remedy-ui-widgets-term` | enums: CursorShape, MouseAction, MouseButton, MouseEncoding, MouseMode, PromptMark, Scrollback, TermColor, Underline; structs: CursorState, Modes, TerminalOptions |

So apps (and the engine's own tests) can still make these types:

- `StylePatch::new()` with a setter per property (`.background(c).radius(6.0)`); 187 struct
  literals across the workspace moved to it.
- A setter per field on every options type: `CodeOptions`, `RichEditorOptions`,
  `RemoteOptions`, `PlayerOptions`, `LadderOptions`, `SpellOptions`, `ParagraphOptions`,
  `BarsOptions`, `MeterOptions`, `RingOptions`, `SparklineOptions`, `CarouselOptions`,
  `EncodeOptions`, `QrOptions`, `RatingOptions`, `SwatchOptions`, `NumberOptions`,
  `ChordOptions`, `FretboardOptions`, `NoteStaffOptions`, `TabOptions`, `TerminalOptions`,
  `ImportPolicy`, `WebConfig`.
- Constructors: `PointerEvent::new` and `with_*`; `KeyEvent::press` / `release` and `with_*`;
  `WheelEvent::new`; `PinchEvent::new`; `TrayIcon::new`; `Decoded::new`; `Release::new`;
  `AudioFormat::new`; `VideoInfo::new`; `Placement::new`; `Default` for `TextInputState`;
  `Modifiers::{SHIFT, CTRL, ALT, META}`.
- Matches the engine repeated on growing enums became methods of the type:
  `Density::name` / `from_name` (theme files), `a11y::Action::name` (the agent protocol).
- Other crates' matches gained wildcard arms that do the right thing for a kind newer than
  them (a terminal sends a newer key's text; an accessibility action the view does not know
  is reported as not performed; a decode failure the sandbox protocol has no code for crosses
  as malformed input with its description).

**Breaking for FreeSpeak's tests:** `tests/settings.rs` builds `PointerEvent` and `KeyEvent`
literals; they become `PointerEvent::new(phase, at).with_buttons(b)` and
`KeyEvent::press(key).with_modifiers(m).with_text(t)`.

## Nothing public by accident, and dependencies in the API

- `remedy-ui-core::testing` (the engine's scenario harness) is `publish = false`; apps test with the new
  `remedy_ui::testing`.
- Dependency types in public signatures were each checked. Kept, as deliberate re-exported
  interop versioned with RemedyUI (STABILITY.md): `wgpu` in `render-gpu` / `embed-gpu`,
  `serde_json` in `embed-webview`, `accesskit` and `raw_window_handle` in `host`, `winit` in
  `bridge-winit` (engine-internal, for the Android bridge), and `serde_json` in `script` (the
  values settings, host data and button handlers are made of; re-exported as
  `remedy_ui::script::serde_json`).
- The remaining public items that exist for other engine crates (the runner-facing methods of
  `Ui`, `media_image::spawn_job`, the host contract) are documented as such; they are the
  contract between the facade, the bridges and the renderers, which STABILITY.md classes as
  engine crates.

## Scripted widgets

The earlier `remedy-ui-engine::script` (one built-in language) was replaced by a language-neutral interface and two engine
crates. The handwritten engine is now `remedy-ui-engine::script_lua` (RemedyLua); WebAssembly is
`remedy-ui-engine::script_wasm`. Facade features are `script-lua` and `script-wasm`, modules
`remedy_ui::script`, `script_lua`, `script_wasm`. Rhai (`script-rhai`, `script_rhai`,
`Language::RHAI`, `Package::rhai`) was a placeholder and is not shipped. `migrations.toml`
renames the mechanical paths; the rest of that break is in the changelog. The changes, with
their replacements:

| Crate | Old | New |
|---|---|---|
| `remedy-ui-engine::script` | `struct ScriptWidget` with `ScriptWidget::load(source, limits)` | `trait ScriptWidget`; widgets load through an engine: `LuaEngine::new().load_source(source, limits)`, `WasmEngine::new().load_module(bytes, limits)`, or any enabled engine with `Engines::load(&Package, limits)` |
| `remedy-ui-engine::script` | `Limits { memory, time }` literals | `#[non_exhaustive]`; `Limits::default()` or `for_device`, then a setter per field (`memory`, `time`, `operations`, `call_depth`, `stack`, `nesting`) |
| `remedy-ui-engine::script` | `SettingSpec { .. }` literals | `#[non_exhaustive]`; `SettingSpec::new(key, label, kind, default)` |
| `remedy-ui-engine::script` | `ScriptError`, `SettingKind` exhaustive | `#[non_exhaustive]`; `ScriptError` gains `Denied`, `OutOfFuel`, `TooDeep` |
| `remedy-ui-engine::script` | (new) | `Language`, `Package`, `Manifest`, `Described` (`#[non_exhaustive]`), `ScriptEngine`, `Engines`, `manifest`, `describe`, `resolve_settings`; `StepRate` and `calibrate_with` (each engine's steps a millisecond, measured on the device) |
| `remedy-ui-engine::script_lua`, `remedy-ui-engine::script_wasm` | (new) | `rate()`, `measure()`, `calibrate()`, and `FALLBACK_STEPS_PER_MS` / `FALLBACK_FUEL_PER_MS` for the moment before the first measurement |
| `remedy-ui-engine::script` | `Node`, `TextStyle` | kept exhaustive, with a comment: the element protocol is a closed set every host builds in full |
| `remedy-ui-widgets::board` | `ScriptKind::new(id, source, limits)` | `ScriptKind::new(id, Package, &Engines, limits)` |
| `remedy-ui-widgets::board` | `Context` | `#[non_exhaustive]` (made with `Context::local`), gains `engines`: the languages the board accepts |
| `remedy-ui-widgets::board` | `build_node(ui, node, &Rc<RefCell<ScriptWidget>>, version, open)` | `build_node(ui, node, &SharedWidget, version, open, stopped)`, `SharedWidget = Rc<RefCell<Box<dyn ScriptWidget>>>`; `stopped` takes a handler's stop (`FailurePhase`, `ScriptError`) |
| `remedy-ui-widgets::board` | a stopped script showed a label (a handler's stop a toast) | each scripted widget is a `catch_errors` boundary: a stop goes to `Ui::failures` and the error hook, and the widget shows why with a "Start again" button (its `Retry`) |

## Reachable through the facade

From the documentation work's findings:

- `remedy_ui::a11y` and `remedy_ui::paint` are re-exported, so apps can name `Semantics`,
  `Role`, `Image`, `Affine`, `Brush` and `Path` without adding crates.
- `remedy_ui::Store` (`#[derive(Store)]`, feature `macros`) and `remedy_ui::isolated`
  (feature `isolated`). The derive finds how the crate depends on RemedyUI
  (`proc-macro-crate`), so its code names `remedy_ui::reactive` for facade users.
- Compiled `.rui` code reaches the runtime through one `use` of a root path:
  `remedy_ui::markup` by default; `codegen::compile_in` / `build::compile_dir_in` take another
  (`DIRECT_ROOT` for apps on `remedy-ui-widgets::rui` itself). A facade test compiles markup through it.
- `remedy_ui::testing::TestApp` (features `headless` and `widgets`): opens an app on the
  in-memory host and finds, presses and types into elements by role and accessible name. The
  `cargo remedy new` templates' tests use it.
- `remedy_ui::Edition`, `BackgroundJob`, `Cancelled`.

## New API from the documentation work's findings

| Where | What |
|---|---|
| `widgets-core` | `button_dyn`: a button whose label follows signals (and what screen readers read) |
| `widgets-nav` | `nav_item_dyn` (`nav_item` builds on it) |
| `widgets-nav` | `Commands::export_bindings` / `import_bindings`, `bindings_to_json` / `bindings_from_json`: the shortcuts file format, shared with `customize` |
| `view` | `TextField::submit_on_enter` works for plain multi-line fields |
| `remedy-ui` | `App::tray_dyn`; `App::background` returns a `BackgroundJob` that cancels, and `App::background_cancellable` hands the work a `Cancelled` |
| `remedy-ui` | `Window::attach_agents` / `Window::agents` (feature `agents`) |
| `automation` | `Agents::request_access`: the standard consent prompt on the trusted layer; the token is handed over only on Allow |
| `cargo-remedy` | `document` and `android` templates; `package --target linux` (`.tar.gz` with a desktop entry), `--target android` (signed APK without Gradle), `--target macos` (`.app` in a zip; needs Apple's linker, so not built on this machine); `migrate` |

### Engine features added before the freeze, under the same conventions

| Where | What | Conventions |
|---|---|---|
| `reactive` | `provide_context`, `use_context`, `with_context`, `Owner`; `Scope::detached`, `Effect::new_in`, `Reaction::new_in` | — |
| `reactive` | `ErrorSink`, `Caught` (`#[non_exhaustive]`), `add_panic_handler`, `panic_message` | — |
| `reactive` | `Resource`, `Executor`, `Job`, `WorkerPool` (on `core::exec`), `Cancel`, `Suspense` | `FetchError` (`#[non_exhaustive]`, implements `Error`) |
| `reactive` (feature `persist`) | `Signal::persist`, `persist_in`, `PersistStore`, `Persist` | `PersistError` (`#[non_exhaustive]`); migrations return `MigrationError` (`From<String>`/`From<&str>`), not `String` |
| `view` | `show` → `Show`, `catch_errors`, `suspense` → `SuspenseBoundary`, `Transition` (`#[non_exhaustive]`, `new`/`fade`/`slide`), `Retry` | `CaughtError` and `FailurePhase` `#[non_exhaustive]`, `FailurePhase::describe` for what a match would spell out |
| `view` | `Ui::failures`, `clear_failures`, `retry_failure`, `is_failed`, `set_error_hook`, `report_failure` | booleans read `is_*` |
| `view` | `El::provide`, `use_context`, `child_with`, `scope`, `on_accessibility_action`, `prop` | `on_*` for callbacks |
| `view` | `El::animate`, `animate_then`, `animate_style`, `set_animated`, `flip`, `drawn_offset`, `is_animating`; `motion::Target`, `Pace` (`#[non_exhaustive]`); `PaintCx::time`, `reduced_motion` | facade path `remedy_ui::motion` |
| `view` | `IntoProp`, `Prop` (exhaustive on purpose: set once or bound) | — |
| `style` | `Space`, `Radius`, `TextRole`, `ColorRole`, `Dp` (`#[non_exhaustive]`), `ColorRef`, `Pad` (exhaustive on purpose), `Style` (`#[non_exhaustive]`, builder) | — |
| `widgets-core` | `ListItems` (`#[non_exhaustive]`), `ListTransitions` (`#[non_exhaustive]`, builder), `for_each_with` | lists treat a `VecDiff` they do not know as a whole-contents change |
| `macros`, `devtools` | `#[preview]`, `#[previews]`, `PreviewSpec` (exhaustive on purpose: the macro writes it as a literal), `golden_test` | errors are `GoldenError` |
| `widgets-term` | `Emulator::take_clipboard_requests` | — |
| `remedy-ui` | `App::persist_store` | — |

## Polish from the documentation guides

The guides' writer met these while writing; each is fixed at its root.

| Where | Was | Now |
|---|---|---|
| `remedy_ui::testing` | `TestApp` used the app's real data folder: every window made `<AppData>/<name>/state` for persisted signals | A private temporary folder, removed when the `TestApp` drops, unless the test sets `AppConfig::data_dir`; `TestApp::data_dir` says which |
| `remedy_ui::testing` | `find` / `press` matched each element's own label, so list rows (which merge their contents into one name) were never found | They match the accessible name screen readers get (`Ui::reader_view`): by role and name, merged names included, merged-away parts not found on their own; `TestApp::reader_view` |
| `remedy_ui::testing` | `key` sent no release; no way to read a node's state | `key` presses and releases; `key_press` / `key_release` separately; `node(role, name)` returns the `ReaderNode`: value and `checked`, `selected`, `expanded`, `disabled`, `focused` |
| `remedy-ui-core::a11y` | `ReaderNode` had states only as words | `checked`, `selected`, `expanded`, `disabled` fields (it is `#[non_exhaustive]`, so additive) |
| `remedy-ui` | `show`, `catch_errors`, `suspense` only at `remedy_ui::view::*` | Also `remedy_ui::{show, catch_errors, suspense}` with `Show`, `SuspenseBoundary`, `CaughtError`, `Retry`; `catch_errors` documents (with an example) that its content is a builder closure, rebuilt on retry |
| `widgets-data` | `chat_list` took only `Signal<Vec<T>>` | `impl Into<ListItems<T>>` like the other lists: `Signal<Vec<T>>` or `SignalVec<T>`; `ListItems::get` is the subscribing read it uses |
| `style` | `Space` stopped at `S8` with no half step | The design's scale (design 13, "Spacing scale"): half steps `S0_5` … `S3_5`, whole steps to `S8`, layout steps `S10`, `S12`, `S16`, `S20`, `S24`; `units()` is `f32`; `ALL`, `name`, `from_name`; every step resolves through the theme's unit |
| `doc-rich` | `markdown(&str)` and `markdown_dyn(closure)` | One `markdown(ui, impl IntoProp<String>, open)`; a constant builds once, a signal, memo or closure rebuilds when the text changes. No app used `markdown_dyn`, so it is removed (STABILITY.md: aliases only for names apps use) and `cargo remedy migrate` renames calls |
| `remedy-ui`, `bridge-web` | `remedy_ui::web` existed only on wasm32, so desktop documentation could not link it | `remedy_ui::web` (feature `web`) on every target: `WebConfig`, `Mount`, `convert`, `files` are platform-neutral, and `run`, `run_with`, `run_single_instance`, `launch_args`, `is_running` are documented on every target (`cfg(any(wasm32, doc))`; in documentation for another target they answer that they run only in a page). Native builds compile no web code: the browser bindings stay wasm32-only dependencies of the bridge |
| `remedy-ui` | Apps could not choose where their window mounts | `AppConfig::web(WebConfig)` (feature `web`, every target, used in a page): `remedy_ui::run` starts the page with it, single instance included (`run_single_instance` takes the config) |
| `cargo-remedy` | `tools/build-web.sh` built only the bridge's own examples | `cargo remedy package --target web [--example <name>] [--dev] [--out <dir>]` builds any app crate into an installable page: the wasm32 build with `remedy-ui/web`, wasm-bindgen's glue (checked against the version in the app's `Cargo.lock`), `index.html`, a manifest, 192/512 px icons (or an SVG), a service worker caching exactly this build under a per-app cache name, and a zip. `[web]` in `remedy.toml` sets the short name, theme color and icon. `build-web.sh` now calls it; a test packages a generated app when the target and CLI are installed |
| `widgets-core` | A trusted prompt's button delay started when its timer request was handled: built outside an event it started at the next frame, built inside one before it showed, and in a covered or minimized window it ran out unseen | The delay starts when the prompt is first seen (`on_visibility`, after the frame that shows it), and starts over if the window is covered or minimized and shown again, so the unpressable moment is never shortened or skipped |

The guides (`remedy_ui::guide`, whose samples are doctests) and recipes
(`crates/remedy-ui/examples`, tested by `tests/recipes.rs`) use the polished API: `TestApp::node`
and the private data folder in the testing guide (the recipe tests no longer make their own
data folders), merged rows found by name in the accessibility guide, `remedy_ui::{show,
catch_errors, suspense}` everywhere, `markdown` over a signal, `AppConfig::web` and `cargo
remedy package --target web` in the web and packaging guides, the spacing scale's half steps
in the theming guide, and the chat feed recipe on a `SignalVec`.

## Bugs the review found

- **Undo grouping never saw time pass.** `Ui::set_time` had no caller, so the view's edit clock
  stayed at zero and a pause in typing never started a new undo step. The runner now sets every
  window's clock from the host before each event (`Ui::set_clock`); a facade test types, waits
  three seconds, types, and undoes only the second burst.
- **Assistive technology could press disabled controls.** `Ui::accessibility_action(Click)`
  checked only that an element had a click handler. A trusted prompt keeps its buttons
  disabled for a moment so a press aimed at what was there before cannot land on them; that
  protection did not hold for screen readers (or anything driving them). Clicks on disabled
  controls are now refused, as pointer clicks are.

## Documentation

Every public item was documented already (the workspace denies `missing_docs`). The review
added runnable examples to the entry points it touched: `Edition`, `Ui::edition`,
`AppConfig::edition`, `StylePatch`, `PointerEvent`, `KeyEvent::press`, `text`, `button_dyn`,
`Rejection`, `BackendError`, `PrepareError`, `DesktopError`, `Density::from_name`,
`Action::name`, `bindings_to_json`, `App::tray_dyn`, `App::background_cancellable`,
`Window::attach_agents`, `Agents::request_access`, `remedy_ui::testing`.
