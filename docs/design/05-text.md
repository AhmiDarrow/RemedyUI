# 5. Text

Every app is mostly text. RemedyUI's text has to be as correct as a browser's, as crisp as native DirectWrite or Core Text, and as fast as a game's. It must also edit text as well as a word processor. The stack is pure Rust, safe and fuzzed:

- `skrifa` and `read-fonts` for fonts and hinting
- `harfrust` for shaping
- ICU4X for segmentation and bidi
- `parley` for paragraph layout, kept behind RemedyUI's own types

## Pipeline

```
text + spans ─▶ itemize ─▶ shape ─▶ break lines ─▶ position ─▶ glyph runs ─▶ rasterize (atlas) ─▶ draw
                (script, bidi level, font fallback, language)      (UAX #14)   (align, justify, bidi reorder)
```

1. **Itemize.** The text is split into runs by bidi level (UAX #9), script, language, span style and font. Font fallback picks, per run, the first face in the family's chain that covers the characters, keeping grapheme clusters whole.
2. **Shape.** `harfrust` turns each run into positioned glyphs with the span's OpenType features and variation axes. Shaped runs are cached by (text, face, size, features, axes, language), so the same label in a thousand rows is shaped once.
3. **Break.** Line breaks follow UAX #14, with hyphenation dictionaries per language when the span asks for it. Paragraphs can be balanced (headings) or pretty (no lone last word).
4. **Position.** Lines are aligned (start, end, center, justify), with bidi reordering per line. Inline objects take their measured size and baseline.
5. **Rasterize.** Each glyph is drawn into the atlas at the exact pixel size, with hinting and subpixel mode as configured, and cached by (glyph, size, mode, subpixel offset).

## Paragraphs and rich text

- **`RichText`** is the model: a string plus spans (font, weight, width, slant, size, color, decorations, letter spacing, OpenType features, link, custom attributes) and inline objects (any element, sized by layout, placed on the baseline or centered).
- **One paragraph, many styles.** A FreeSpeak message with bold, a link, a spoiler and custom emoji is one paragraph. It wraps inside runs and around inline objects.
- **Builders** read naturally:

  ```rust
  rich_text().text("Meet at ").bold("10:30").text(" in ").link("#general", go_general).text(" ").inline(emoji(":wave:"))
  ```

- **Inheritance.** Paragraph style is inherited from context (font, size, color, line height), as CSS and Flutter do. A `text("...")` inside a button gets the button's style without app code.
- **Tabular figures, small caps and stylistic sets** are span features. Remedy Sans UI's tabular figures are on by default in numeric contexts (counters, clocks, tables).

## Fonts

- **The family registry.** A family is a list of faces (weights, widths, styles, variable axes) plus a fallback chain. Apps register their own families from bytes or files. The Remedy families come as a crate (`remedy-ui-fonts`), each family a feature, so an app that never uses AhmiPixel does not carry it.
- **System fonts are opt-in** per family as a last fallback: the platform's emoji font, or CJK fonts the app does not bundle. They are looked up through the bridge (DirectWrite, Core Text, fontconfig, Android `FontMatcher`).
- **Variable fonts.** Optical size follows the pixel size automatically. Weight and width are continuous, so an animation between weights is smooth.
- **Color fonts.** COLRv1 is painted with RemedyUI's own gradients and clips. COLRv0, CBDT and sbix are drawn into the color atlas.
- **Loading.** Fonts are memory-mapped and parsed per face on first use. A font from an untrusted source (a theme, a user file, a message attachment) is parsed by `read-fonts` only, which is fuzzed and bounds-checked. A face that fails to parse is skipped in fallback, with a log line, never a crash.

## Sharp text

| Setting | What it does | Default |
|---|---|---|
| Hinting `full` | TrueType bytecode or the CFF hinter at the pixel size; stems on whole pixels; one subpixel position | Windows, Linux at ≤ 1.5× scale |
| Hinting `light` | Vertical-only autohinting; four horizontal subpixel positions | Linux and Android at > 1.5× scale |
| Hinting `none` | Exact outlines; four subpixel positions | macOS, and all text above 48 px |
| Antialiasing `lcd` (RGB, BGR, VRGB, VBGR) | Three-times horizontal coverage with the FreeType default five-tap filter, per-channel blending | Windows when ClearType is on, read from the system |
| Antialiasing `grayscale` | One coverage channel | Elsewhere, and over images, video, transforms or transparent layers |

- **Contrast and gamma** follow each platform's native text so RemedyUI text sits beside system text without looking heavier or lighter.
- **Pixel-aligned lines.** When hinting is on, line metrics are rounded to whole pixels and each line's baseline is snapped. Text does not shimmer as a list scrolls by fractional amounts, because the scroll offset is applied to whole layers, not to glyph positions.
- **Large and animated text** (above 48 px, or under a non-translation transform) is drawn as paths, not atlas bitmaps, so it stays sharp when zoomed or rotated.

## Selection

- **`selectable()` on any text.** Drag, double-click for a word, triple-click for a paragraph, Shift+click and Shift+arrows to extend, Ctrl+A to select all in the current scope.
- **Selection across elements.** A selection scope (a chat history, a document, a settings page) holds an ordered set of selectable texts. Dragging across them selects in reading order, including bidi order inside each paragraph. Copy produces plain text, and also rich text (HTML on the clipboard) when the scope asks for it.
- **Virtual lists** keep their selection on the data model, so selecting from message 10 to message 5,000 works while most rows are unmounted.
- **Handles and the context menu** appear on touch devices, with the platform's magnifier where it has one.

## Editing

`TextInput` and `RichEditor` share one editing core:

- **Model.** A rope for large documents. Edits are incremental: only the changed paragraphs are re-itemized, re-shaped and re-broken.
- **Cursor movement** respects grapheme clusters, bidi visual order (arrow keys move visually), word boundaries by locale, line and paragraph ends, and page up and down.
- **Undo and redo.** Edits are grouped by typing pauses and by word, and IME commits are atomic. The history joins the engine's undo service, so Ctrl+Z works the same in any field and in app-level actions.
- **Input methods.** These follow each platform's real protocol, not a lowest common denominator:
  - **Windows:** Text Services Framework (TSF), with full composition, reconversion, surrounding text, and the touch keyboard and emoji panel (Win+.).
  - **macOS:** `NSTextInputClient`, with marked text, the candidate window at the caret, and dictation.
  - **Linux:** Wayland `text-input-v3`, or IBus and Fcitx on X11.
  - **Android:** `InputConnection` with composing regions, autocorrect and suggestions.
  - **RemedyOS:** the native text-input wire.
  - In every case, preedit text is drawn inline with the platform's underline styles.
- **Rich editing.** `RichEditor` edits `RichText`. It handles span toggles (bold and the rest), links, lists, quotes, code, and inline objects as atomic characters. Formats plug in through a serializer trait: Markdown for FreeSpeak, an app's own format for notes. Paste is sanitized: HTML comes in as allowed spans only and never as markup that runs.
- **Spell checking** (`services-spell`) underlines misspellings using the platform checker and offers suggestions and "add to dictionary" in the context menu. It checks only text the user typed, never passwords, and only on the device.
- **Password fields** are masked, with optional reveal. Their buffers are zeroed on change and drop, never exposed as text to accessibility or automation, never copied by the engine, and excluded from undo history and IME learning where the platform allows (`IS_PRIVATE`, `kTISPropertyInputSourceIsASCIICapable`-class hints).

## Robustness and attacks through text

- **Shaping cost** is kept proportional to what can be seen. Pathological input (thousands of combining marks, "Zalgo" text) is limited per grapheme cluster to what renders differently, and the rest is kept in the text but not stacked. A message of any length is laid out lazily, line by line, as it scrolls into view.
- **Bidi spoofing.** Invisible bidi controls (Trojan Source class) are shown as visible marks in links and in text marked `security_sensitive`. Link text whose shown domain differs from its target is flagged through the app's link hook.
- **Confusables.** A UTS #39 skeleton check is available for usernames and domains, so apps can warn about look-alike names.
