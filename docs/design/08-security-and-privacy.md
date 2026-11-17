# 8. Security and privacy

The Remedy apps hold encrypted vaults (SecretFolder, SecretSticky), private conversations (FreeSpeak, Hearth), microphone audio (VoiceRemedy, GuitarRemedy) and an AI agent that can act on the computer (the RemedyAI desktop, the RemedyOS mind). A UI engine sits between all of that and the screen, keyboard and clipboard, so it is part of each app's security.

## Threat model

| From | Examples | Defended by |
|---|---|---|
| Content from other people | Messages, images, GIFs, fonts, documents' embedded assets, shared themes and layouts, link text | Memory-safe parsers off the UI thread, bounds from need, data-only formats, spoofing checks |
| Other local programs | Screen capture, clipboard monitors, accessibility snooping, input injection | Capture exclusion, sensitive clipboard, protected semantics, source-checked input for secure prompts |
| Over-reaching agents | An AI agent operating the UI beyond what the user allowed | Consent-gated automation through commands only, visible and undoable (see [11](11-accessibility-and-automation.md)) |
| The engine's own bugs | Memory errors, panics, resource exhaustion | `forbid(unsafe_code)`, no-panic frame path, fuzzing, sizing from need |
| The supply chain | A compromised or abandoned dependency | Few dependencies, pinned versions, `cargo deny` / `cargo vet` / `cargo audit`, everything behind internal types so it can be replaced |

## Memory safety

- `#![forbid(unsafe_code)]` in every crate except `render-gpu` and the bridges. Those deny `unsafe_code` at the crate root and allow it only on one module (`ffi` in `render-gpu`, `os` in each bridge). The modules are reviewed per change, run under Miri where the platform allows, and wrap OS calls in safe types with documented invariants. Packs that need the operating system go through bridge traits.
- **The frame path** denies `unwrap`, `expect`, panicking indexing and unchecked arithmetic, enforced by Clippy in CI. Overflow checks stay on in release.
- **App code** runs inside `catch_unwind` at every callback boundary, with error boundaries in the tree ([02](02-state-and-elements.md)).

## Untrusted bytes

- **Parsers.** Everything from outside is parsed by pure-Rust, fuzzed parsers: images, animations, SVG, fonts, theme files, `.rui` overrides, rich-text paste.
- **Off the UI thread.** Parsing runs on the worker pool, never in a frame.
- **Bounds from need, not fixed caps.**
  - An image is decoded only up to the largest size it can be shown at: its element's size × the window's scale, or the display's size for a full-screen viewer. A 30,000 × 30,000 image shown as a 40 dp avatar decodes to 40 dp: JPEG through scaled decoding, row-by-row formats (PNG, WebP lossless) by downsampling rows as they stream, so there is no full-size buffer. Formats that need the whole image at once (interlaced PNG, some TIFF layouts) are decoded at full size only if the machine's free memory allows it; otherwise they show their error state.
  - Animations decode frames as they are needed, at the shown size, not all up front.
  - Text is laid out as it becomes visible.
  - Work that cannot finish while still useful is cancelled when its element leaves the screen.
  - These bounds follow the content and the hardware. There is no magic number to raise later.
- **Isolated decoding** (optional, `media-isolated`). For apps that want defense in depth, decoding runs in a separate, sandboxed helper process that receives bytes and returns pixels over shared memory. The sandboxes are an AppContainer with a restricted token on Windows, seccomp-bpf plus Landlock on Linux, the App Sandbox on macOS, an isolated process on Android, and a kernel isolation domain on RemedyOS. A crash or exploit in a decoder takes down only the helper, which restarts. This is the Chromium model, used where it matters (FreeSpeak, Hearth, RemedyPDF's embedded images).
- **Data, never code.** Themes, layouts, message files and rich text are data. They cannot run code, load arbitrary files, open network connections or include shaders unless the user turns that on per item. Paste of HTML produces allowed spans only.

## Secrets on screen

The `secure` crate (L3) and the zeroizing secret buffer in `text`, used by SecretFolder, SecretSticky, Hearth's passphrase and FreeSpeak's safety numbers:

- **`secret_input`.** Masked text with optional reveal.
  - Its storage zeroes on change and drop, and keeps one contiguous buffer to avoid copies.
  - It is never exposed as text to accessibility or automation.
  - It is excluded from undo, spell checking, IME learning (platform private-input hints) and the engine's logs.
  - Paste in is allowed, copy out is not.
- **Capture exclusion.** A window or element marked `protect_from_capture()` is excluded from screenshots, screen recording and screen sharing: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on Windows, `NSWindow.sharingType = .none` on macOS, `FLAG_SECURE` on Android, the kernel capture policy on RemedyOS. Where only whole windows can be protected, a protected element moves to its own child window.
- **Sensitive clipboard.** Copying with `sensitive()` keeps the item out of history and sync where the OS allows (`ExcludeClipboardContentFromMonitorProcessing` and cloud-clipboard exclusion on Windows, `org.nspasteboard.ConcealedType` on macOS, `EXTRA_IS_SENSITIVE` on Android). It also clears the clipboard after a set time, only if it still holds the same content.
- **Idle lock.** An idle signal fed only by real user input (pointer, key, touch), never by timers or programmatic events, drives auto-lock. The lock screen is a modal layer that takes all input, hides the content below from accessibility, and can blur or remove that content from the frame.
- **Privacy screen on switch.** On app switch or minimize, a marked window can show a cover instead of its content in the OS task switcher.

## Spoofing and deception

- **Links** report their real target. Hover and focus show it, accessibility announces it, and the app's link hook can mark a link risky (FreeSpeak's registrable-domain check) so the link shows a warning style.
- **Bidi and invisible characters** in links, usernames and `security_sensitive` text are shown as visible marks (Trojan Source class).
- **Look-alike names.** UTS #39 confusable skeletons are available to compare usernames and domains.
- **Overlays.** Nothing an app draws can cover a permission or approval prompt shown through the engine's `trusted_prompt` layer. That layer is always on top, ignores synthetic and agent input, and needs a deliberate press: a short delay before its buttons enable, as browsers use against click-jacking.

## Privacy by construction

- **No telemetry.** The engine sends nothing over the network. The single networked pack, `services-update`, is opt-in, contacts only the feed the app configures, and sends no identifier beyond what an HTTPS request carries. Crash and error hooks give data to the app, which decides.
- **Logs** hold element paths, timings and error kinds, never text content, input or clipboard data. Debug content logging is a separate opt-in that cannot be on in release.
- **Microphone, camera and screen capture** are not engine features. When an app uses them through its own code, the engine provides the standard indicator component and the platform's permission prompt bridge, so users always see when capture is live.

## Supply chain and process

- **Dependencies.** Every third-party crate is justified in `docs/dependencies.md`, pinned exactly, under one of the licenses listed there, and checked by `cargo deny` (licenses, advisories, bans, sources), `cargo audit` and `cargo vet` in CI. Features are minimal per dependency.
- **Fuzzing.** Fuzz targets (cargo-fuzz) run in CI on a schedule, with the corpus committed: rich-text building, the editor, `.rui` and theme parsing, image and font paths, the display list, and the layout of random trees.
- **Reproducible release builds**, with signed tags.
- **Security issues** are reported privately and fixed under an advisory before disclosure. Their regression tests become part of the suite.
