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

- **`RichText`** is the model: a string plus spans (font, weight, width, slant, size, color, decorations, letter spacing, OpenType features, link, a filled background, custom attributes) and inline objects (any element, sized by layout, placed on the baseline or centered). A span background is a chip: its inline padding is laid out so the fill does not cover the words beside it, and its block padding and corner radius are painted.
- **One paragraph, many styles.** A FreeSpeak message with bold, a link, a spoiler and custom emoji is one paragraph. It wraps inside runs and around inline objects.
- **Builders** read naturally:

  ```rust
  rich_text().text("Meet at ").bold("10:30").text(" in ").link("#general", go_general).text(" ").inline(emoji(":wave:"))
  ```

- **Inheritance.** Paragraph style is inherited from context (font, size, color, line height), as CSS and Flutter do. A `text("...")` inside a button gets the button's style without app code.
- **Line clamping.** `ParagraphOptions::max_lines(n)` (`El::max_lines`, `Style::max_lines`) lays out starts of the text growing fourfold until one reaches past line `n + 1` (lines break greedily, so its first lines are the whole text's), then binary-searches the grapheme boundaries of line `n` for the longest prefix that still fits in `n` lines with "…" after it. The "…" is a neutral character at the logical end, so the bidi algorithm puts it on the line's visual end (the left in right-to-left text). The cut is a prefix, so offsets stay those of the text; readers and copying get the whole text.
- **Text transforms.** `TextTransform` (style or span) cases the text before shaping, by the engine's locale (Turkish and Azerbaijani i, ß as SS, Greek final sigma, Greek capitals without accents as ICU's Greek upper case, Dutch IJ, title-case digraphs). The paragraph keeps a map from the cased text's offsets to the original's and answers hit tests, carets, selections, lines and glyph clusters in the original's, so everything outside the shaper sees the text as written.
- **Tabular figures, small caps and stylistic sets** are span features. Remedy Sans UI's tabular figures are on by default in numeric contexts (counters, clocks, tables).

## Huge paragraphs

A paragraph of 32 KB or more that wraps (`LAZY_MIN_BYTES`) is laid out lazily: in pieces of about 8 KB (`LAZY_CHUNK_BYTES`), at most 48 of them kept laid out at once (`LAZY_KEPT_PIECES`; the rest are laid out again when asked for), each shaped and broken by parley on its own, only when something asks for its lines. A view asks for the band that shows and a window's height on each side; carets, selections, hit tests and movements ask for the lines they reach. Whatever is laid out is exactly what a whole layout gives (the same line breaks, glyphs and positions, hit tests, carets and selections), which the tests check line by line and glyph by glyph in Latin, mixed Arabic, Hebrew and English in both base directions, and Chinese.

- **Checkpoints.** Lines break greedily, so the lines after a line start depend only on the text from there. Each piece starts at a line start the piece before found, at the unrounded height the lines above add up to (parley rounds each line's top from that sum, so the piece's lines land on the same pixels).
- **Seams.** A piece's last two lines are dropped (the end of its text breaks them), and each piece shares two lines with the one before. Both must lay those lines out identically, glyph for glyph, before the later piece's lines are used: this is where the shaper's and the line breaker's context across the seam would show. Where they differ, the earlier piece grows and takes the seam inside it, up to the whole paragraph.
- **Bidi bounds the laziness.** UAX #9 resolves levels per paragraph: a number's level can depend on the last strong letter however far back, a bracket's on its pair however far ahead. Text before the first character that needs resolution at all (right-to-left letters, Arabic numbers, explicit embeddings, overrides and isolates), in a left-to-right paragraph, has level 0 whatever follows, so pieces there need nothing more; the paragraph is scanned only as far as it is laid out. Past such a character, or in a right-to-left paragraph, the whole paragraph's levels are resolved once (parley's own analysis, linear and about a sixth of the cost of laying it out) and each piece's levels must match them on the lines it keeps, the piece growing until they do. So a paragraph with right-to-left text pays one bidi pass over all of it before its first lines and after each edit, and a run of text whose levels depend on context a piece does not have is laid out in one piece. The base direction is found once, as parley finds it, and given to every piece.
- **Limits.** The three bounds are the app's to move (`text::set_lazy_limits` with a `LazyLimits`); a paragraph keeps the piece size and pieces kept it started with, so a change never mixes two piece sizes in one layout.
- **Heights** below the last line laid out are estimated from the lines laid out so far; a view measures the element again as they firm up. Lines above what shows are always laid out, so the estimate never moves what is on screen. Jumping far ahead lays out the lines in between once.
- **Edits.** Laying out the edited text from the paragraph before it keeps the pieces before the edit and, once a line start after it is the start of a piece from before and the two agree on that piece's first lines, the pieces after it, moved by the edit's length and height. Typing in the middle of a 10 MB paragraph lays out about one piece.
- **Memory.** Pieces far from what was asked for last are dropped and laid out again, to the same lines, when needed.
- A paragraph that does not wrap is one line and is laid out whole.

## Fonts

- **The family registry.** A family is a list of faces (weights, widths, styles, variable axes) plus a fallback chain. Apps register their own families from bytes or files. The Remedy families come as a crate (`remedy-ui-fonts`), each family a feature, so an app that never uses AhmiPixel does not carry it.
- **System fonts are opt-in** per family as a last fallback: the platform's emoji font, or CJK fonts the app does not bundle. They are found by `fontique`, inside the text crate, which reads each platform's font list; on RemedyOS the bridge supplies the font list.
- **Symbol fallback.** Fallback chains are per script, and symbols (✦ ◆ ⚠ ☰ ⌕ ⇄ …) are in Unicode's Common script: they take the script of the text around them, whose fallback fonts rarely have them. So a character that still has no glyph after its style's families and its script's chain gets a second try, for that character only, with the collection's symbol fallbacks after its families: the bundled Remedy Symbols (a 9 KB face of interface symbols, drawn from geometry on Remedy Sans UI's metrics, with no letters so it never takes a script's text), then each platform's symbol, math and emoji fonts. Paragraphs with no missing glyph are shaped once, as before.
- **Variable fonts.** Optical size follows the pixel size automatically. Weight and width are continuous, so an animation between weights is smooth.
- **Color fonts.** COLRv1 is painted with RemedyUI's own gradients and clips. COLRv0, CBDT and sbix are drawn into the color atlas. A bundled Remedy Emoji face (our own art, COLRv1) is the default emoji fallback, so the emoji it has paint without a system color-emoji font, and goldens do not change with whichever Noto Color Emoji a machine has installed.
- **Loading.** Bundled fonts are static bytes in the binary; user and system fonts are read into memory when first used. Each face is parsed on first use. A font from an untrusted source (a theme, a user file, a message attachment) is parsed by `read-fonts` only, which is fuzzed and bounds-checked. A face that fails to parse is skipped in fallback, with a log line, never a crash.

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
- **Spell checking** (`services-spell`) underlines misspellings and offers suggestions, "Add to dictionary" and "Ignore" in the context menu. For each language it uses the platform's checker where there is one (Windows `ISpellChecker`, macOS `NSSpellChecker`) and otherwise Hunspell dictionaries found in the usual system folders or given by the app; a language no engine can check is reported, never guessed. Words are cut out by ICU word segmentation, leaving out links, addresses, mentions, numbers, code and acronyms. Checking runs on a thread of its own and the marks arrive through the waker, so typing never waits; the word being typed is marked once typing pauses. Marks are field decorations (`TextField::set_decorations`), which carry over edits made while a check ran. It checks only text the user typed, never passwords, and only on the device; the personal dictionary is the app's to save and load.
- **Password fields** are masked, with optional reveal. Their buffers are zeroed on change and drop, never exposed as text to accessibility or automation, never copied by the engine, and excluded from undo history and IME learning where the platform allows: Windows `InputScope` password scope, Android `IME_FLAG_NO_PERSONALIZED_LEARNING` with a password input type, macOS secure event input.

## Robustness and attacks through text

- **Shaping cost** is kept proportional to what can be seen. Pathological input (thousands of combining marks, "Zalgo" text) is limited per grapheme cluster to what renders differently, and the rest is kept in the text but not stacked. A message of any length is laid out lazily, line by line, as it scrolls into view.
- **Bidi spoofing.** Invisible bidi controls (Trojan Source class) are shown as visible marks in links and in text marked `security_sensitive`. Link text whose shown domain differs from its target is flagged through the app's link hook.
- **Confusables.** A UTS #39 skeleton check is available for usernames and domains, so apps can warn about look-alike names.
