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
`remedy-ui-script` and `remedy-ui-widgets-board` are left as they are on main: they are being
rebuilt on Rhai and WebAssembly engines, and get these conventions when that lands.

**For other branches merging after this one:** every rename below is in
`crates/cargo-remedy/migrations.toml`, so `cargo remedy migrate --path <crate>` applies the
mechanical ones to a branch's own code; the struct-literal and `match` changes are listed
under "Room to grow" with the replacement for each.

## Renames

| Crate | Old | New | Why |
|---|---|---|---|
| `remedy-ui-text` | `StylePatch` | `TextStylePatch` | Two public types were named `StylePatch` (element styles in `remedy-ui-style`, span styles here), both reachable through the facade. One name per concept. |
| `remedy-ui-view` | re-export `TextPatch` | `TextStylePatch` | The same type under a third name. |
| `remedy-ui-bridge-headless` | `HeadlessWindow::{maximized, fullscreen, minimized, frame_requested}` | `is_maximized`, `is_fullscreen`, `is_minimized`, `is_frame_requested` | Boolean states read `is_*` everywhere, as on the facade's `Window` (`is_fullscreen`, `is_maximized`, `is_visible`). |
| `remedy-ui-bridge-headless` | `HeadlessHost::exited` | `has_exited` | Same rule, for an event that happened. |
| `remedy-ui-media-playback` | `Player::{muted, fullscreen}` | `is_muted`, `is_fullscreen` | Pairs `set_muted` / `request_fullscreen` with the facade's `set_*` / `is_*`. |
| `remedy-ui-embed-webview` | `WebView::loading` | `is_loading` | Boolean state. |
| `remedy-ui-view` | `Ui::occluded` | `is_occluded` | Boolean state. |
| `remedy-ui-widgets-term` | `Terminal::monospace` | `is_monospace` | Boolean state. |
| `remedy-ui-view` | `Ui::set_time(ms: u64)` | `Ui::set_clock(Duration)` | The unit was only in a parameter name; the setter now takes a `Duration`. Not mechanical (the argument changes type), so not in the migration table. |
| `remedy-ui-widgets-core` | `label(ui, s, variant)` (deprecated, kept) | `text(ui, s).variant(variant)` | The variant was always required; `text` plus the `variant` builder is the one way, with the variant optional. FreeSpeak uses `label`, so it stays deprecated for a release. `cargo remedy migrate` rewrites the calls. |
| `remedy-ui-rui` | `build::compile_dir_to(dir, out, customs, cargo)` | `compile_dir_to(root, dir, out, customs, cargo)` | Compiled markup names the runtime by a root path (below). |

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
- `*_dyn` for forms that follow signals (`text_dyn`, `style_dyn`, `markdown_dyn`, and now
  `button_dyn`, `nav_item_dyn`).
- Units: durations are `Duration`; the nanosecond fields of trace and devtools types end in
  `_ns`; no other numeric time is public.

## Errors

Every fallible public function, trait method and callback now returns a typed error that
implements `std::error::Error` and `Display`. No `Result<_, String>` is public.

| Crate | Old | New |
|---|---|---|
| `remedy-ui` | `RunError(pub String)` | `#[non_exhaustive] enum RunError { SingleInstance, Bridge, UnsupportedPlatform, NeedsFeature }` |
| `remedy-ui` | `App::open_url(.., FnOnce(Result<(), String>))` | `FnOnce(Result<(), host::Unsupported>)` |
| `remedy-ui-bridge-winit`, `remedy-ui-bridge-web` | `RunError(pub String)` | opaque `RunError` (`new`, `reason()`) |
| `remedy-ui-core` | `exec::SpawnError(pub String)` | opaque (`reason()`) |
| `remedy-ui-devtools` | `Snapshot::to_png / from_png -> Result<_, String>` | `PngError`; `GoldenError` implements `Error` and gains `Png` |
| `remedy-ui-services-desktop` | `autostart::set`, `set_in`, `reveal -> Result<(), String>` | `DesktopError` (`NotFound`, `NoAutostartFolder`, `Io`, `Platform`) |
| `remedy-ui-services-spell` | `SpellBackend::{check_word, suggest}`, `BackendFactory` with `String` | `BackendError` (`From<String>` / `From<&str>`, so back ends keep using `?`); `SpellError::Backend(BackendError)` with `source()` |
| `remedy-ui-embed-gpu` | `GpuContent::prepare -> Result<(), String>` | `PrepareError` |
| `remedy-ui-widgets-core` | validators returning `Result<(), String>` | `Rejection`, the reason an app turns down a person's edit, used by `Column::editable`, `TreeState::set_import`, `MoveBox` and `Form::field` |
| `remedy-ui-widgets-nav` | (new) | `BindingsError` for imported shortcut bindings |
| `remedy-ui-core` | (new) | `ParseEditionError` |

## Room to grow

`#[non_exhaustive]` now marks 139 enums and 126 structs; 40 enums stay exhaustive with a
comment saying why. Per crate:

| Crate | Changed |
|---|---|
| `remedy-ui` | enums: RendererChoice, RendererKind, RunError; structs: AppConfig, WindowSpec |
| `remedy-ui-a11y` | enums: Action, Live, Role; structs: ReaderNode |
| `remedy-ui-automation` | enums: AgentAction, Answer, Refused, Request, Scope; structs: CommandNode, Node, Snapshot |
| `remedy-ui-bridge-android` | structs: AndroidSettings |
| `remedy-ui-bridge-web` | enums: Mount; structs: RunError, WebConfig |
| `remedy-ui-bridge-winit` | enums: Backend, UserEvent, WindowNotice; structs: RunError |
| `remedy-ui-compositor` | structs: FrameReport; kept exhaustive: CompositeOp |
| `remedy-ui-core` | enums: Edition, ParseColorError |
| `remedy-ui-customize` | enums: Area, CustomizeError, RefusalKind, TokenKind, TokenValue; structs: ContrastWarning, ImportPolicy, ImportReport, Notice, Refusal |
| `remedy-ui-devtools` | enums: GoldenError, PngError, ProblemKind, RemoteAnswer, RemoteRequest, Unreachable; structs: FrameSample, Problem, ReaderLine, RemoteBinding, RemoteElement, RemoteFrame, RemoteNode, RemoteOptions, RemoteProblem, RemoteRead, RemoteReaderNode, RemoteUpdate, Slice, UpdateEntry |
| `remedy-ui-doc-code` | enums: DiffLine, Kind; structs: CodeOptions |
| `remedy-ui-doc-pages` | enums: AnnotationKind, Fit, RenderError, Shape, Spread, Tool; structs: PageStats, RenderRequest, SearchStatus; kept exhaustive: Rotation |
| `remedy-ui-doc-rich` | structs: RichEditorOptions |
| `remedy-ui-embed-gpu` | enums: EmbedInput, EmbedStatus; structs: EmbedStats, Gpu, PrepareError, Target |
| `remedy-ui-embed-webview` | enums: BlockReason, NavigationSource, WebViewStatus; structs: Blocked, DownloadRequest, NavigationRequest |
| `remedy-ui-fonts` | enums: Style |
| `remedy-ui-host` | enums: TrayEvent; structs: FileDialog, SystemSettings, TextInputState, TrayIcon, Unsupported, WindowOptions; kept exhaustive: AccessibilityEvent, DialogStarted, DragEvent, DragStarted, DropEffect, HostEvent, ImeEvent, MemoryPressure, ResizeEdge, TrayItem |
| `remedy-ui-input` | enums: Claim, GestureKind, Key, Platform, PointerKind, Scope; structs: ChordError, Clash, Conflict, GestureSettings, KeyEvent, PinchEvent, PointerEvent, ScrollPhysics, WheelEvent; kept exhaustive: Modifiers, PinchPhase, PointerPhase |
| `remedy-ui-layout` | enums: Align, Display, JumpAlign, Justify, Length, Position, Track; structs: LayoutStyle, MeasureInput; kept exhaustive: Available, Axis, Direction |
| `remedy-ui-media-anim` | kept exhaustive: Loops, Opened |
| `remedy-ui-media-image` | enums: DecodeError, Fit, ImageState, Source; structs: Decoded |
| `remedy-ui-media-isolated` | enums: EnableError, Layer; structs: Isolation, Stats |
| `remedy-ui-media-playback` | enums: MediaError, MediaSource, PlaybackState, Sample, SubtitleFormat, UnseenPolicy; structs: AudioFormat, MediaInfo, PlaybackStats, PlayerOptions, TextTrackInfo, VideoInfo |
| `remedy-ui-media-video` | enums: VideoFit; structs: VideoStats |
| `remedy-ui-motion` | enums: Easing, Motion |
| `remedy-ui-paint` | kept exhaustive: Blend, Brush, Item, PathEl, PixelFormat, Sampling, StrokeCap, StrokeJoin, YuvMatrix |
| `remedy-ui-reactive` | enums: NodeKind, ReactiveError, Segment, StoreError, VecDiff; structs: Cause, NodeInfo, PostReport, Stats, Update |
| `remedy-ui-render-cpu` | enums: ImageKey, RenderError |
| `remedy-ui-render-gpu` | enums: GpuError, Presented, Rung; structs: LadderOptions |
| `remedy-ui-rui` | enums: Binding, Bound, BuildError, Kind, SaveError, Space, Value; structs: RuiError |
| `remedy-ui-services-desktop` | enums: DesktopError, KnownFolder |
| `remedy-ui-services-i18n` | enums: Arg, DecimalError, Problem; structs: OverrideProblem; kept exhaustive: Weekday |
| `remedy-ui-services-spell` | enums: Availability, Prepared, SpellError; structs: BackendError, BackendReport, MenuLabels, SpellOptions |
| `remedy-ui-services-update` | enums: Apply, Feed, SignatureStatus, Trust, UpdateError, UpdateState; structs: BannerText, Release, UpdateConfig |
| `remedy-ui-style` | enums: Density; structs: ColorRoles, DerivedTokens, MotionTokens, Radii, Spacing, StylePatch, Theme, ThemeAssets, TypeScale |
| `remedy-ui-text` | enums: Align, Antialias, Hinting, LineHeight, Pseudo; structs: AtlasStats, GlyphRunInfo, LineInfo, ParagraphOptions, TextStyle, TextStylePatch; kept exhaustive: Affinity, Block, Direction, Mark |
| `remedy-ui-themes` | enums: ThemeChoice, TokenError |
| `remedy-ui-tree` | enums: HitMode, Phase, TreeError; structs: Dispatch, Failure |
| `remedy-ui-view` | enums: DecorationKind, FieldKind, TextInput; structs: DirtyRecord, ElementInfo, FrameStats, HistoryItem, Placement; kept exhaustive: Request |
| `remedy-ui-viz` | enums: Range, Zone; structs: BarsOptions, MeterOptions, RingOptions, SparklineOptions; kept exhaustive: Orientation |
| `remedy-ui-widgets-core` | enums: Icon; structs: Rejection |
| `remedy-ui-widgets-data` | enums: Change, SortValue; kept exhaustive: SortDirection |
| `remedy-ui-widgets-extra` | enums: QrError; structs: CarouselOptions, EncodeOptions, QrOptions, RatingOptions, StepperLabels, SwatchOptions; kept exhaustive: Ecc, Mode |
| `remedy-ui-widgets-forms` | enums: SubmitState; structs: NumberOptions |
| `remedy-ui-widgets-music` | enums: Clef, Inlays, MarkKind, NoteValue, StringState, Technique; structs: ChordOptions, FretboardOptions, NoteStaffOptions, TabOptions; kept exhaustive: Step |
| `remedy-ui-widgets-nav` | enums: Edge; structs: CommandInfo |
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

- `remedy-ui-test` (the engine's scenario harness) is `publish = false`; apps test with the new
  `remedy_ui::testing`.
- Dependency types in public signatures were each checked. Kept, as deliberate re-exported
  interop versioned with RemedyUI (STABILITY.md): `wgpu` in `render-gpu` / `embed-gpu`,
  `serde_json` in `embed-webview`, `accesskit` and `raw_window_handle` in `host`, `winit` in
  `bridge-winit` (engine-internal, for the Android bridge). `remedy-ui-script` exposes
  `serde_json::Value` without re-exporting it; that crate is being replaced (above).
- The remaining public items that exist for other engine crates (the runner-facing methods of
  `Ui`, `media_image::spawn_job`, the host contract) are documented as such; they are the
  contract between the facade, the bridges and the renderers, which STABILITY.md classes as
  engine crates.

## Reachable through the facade

From the documentation work's findings:

- `remedy_ui::a11y` and `remedy_ui::paint` are re-exported, so apps can name `Semantics`,
  `Role`, `Image`, `Affine`, `Brush` and `Path` without adding crates.
- `remedy_ui::Store` (`#[derive(Store)]`, feature `macros`) and `remedy_ui::isolated`
  (feature `isolated`). The derive finds how the crate depends on RemedyUI
  (`proc-macro-crate`), so its code names `remedy_ui::reactive` for facade users.
- Compiled `.rui` code reaches the runtime through one `use` of a root path:
  `remedy_ui::markup` by default; `codegen::compile_in` / `build::compile_dir_in` take another
  (`DIRECT_ROOT` for apps on `remedy-ui-rui` itself). A facade test compiles markup through it.
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
