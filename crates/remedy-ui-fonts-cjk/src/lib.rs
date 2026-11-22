//! Noto Sans SC and Noto Sans JP, as fallback faces for RemedyUI's text engine: Chinese and
//! Japanese text draws in a face made for it, never as empty boxes, with no fonts installed.
//!
//! Chinese and Japanese share most of their characters (Unicode's Han unification) but
//! write many of them differently: 直, 角, 骨 and 写 take different shapes in each. Each face
//! is registered for its language ([`script_fallbacks`]), so a Japanese window draws 直 the
//! Japanese way and a Chinese one the Chinese way, and kana take the Japanese face.
//!
//! | Feature | Face | Weights | Covers |
//! |---|---|---|---|
//! | `sc` (default) | Noto Sans SC | Regular, Bold | GB 2312 and some 6,000 more hanzi, kana, punctuation |
//! | `jp` (default) | Noto Sans JP | Regular, Bold | JIS X 0208 and more kanji, kana, punctuation |
//!
//! The faces are WOFF2 (about 2 MB each); the text engine unpacks one once
//! (`remedy_ui_text::FontFile`). In a browser page they are not built into the module but
//! fetched from beside the page the first time Chinese or Japanese text shows (see
//! `remedy-ui-fonts`; the `embed` feature builds them in). Licensed under the SIL Open Font
//! License 1.1.
//!
//! ```
//! for face in remedy_ui_fonts_cjk::all() {
//!     assert!(face.data.starts_with(b"wOF2"));
//! }
//! ```

pub use remedy_ui_fonts::{Face, ScriptFallback, Style};

// Built in natively; in a browser page, a file fetched beside the page when text first needs
// it (unless `embed`), named for `cargo remedy package` (`remedy_ui_fonts::asset!`).
macro_rules! face {
    ($family:literal, $weight:literal, $file:literal) => {{
        #[cfg(any(not(target_arch = "wasm32"), feature = "embed"))]
        let data: &'static [u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/", $file));
        #[cfg(all(target_arch = "wasm32", not(feature = "embed")))]
        let data: &'static [u8] = {
            remedy_ui_fonts::asset!(concat!("fonts/", $file), concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/", $file));
            &[]
        };
        Face { family: $family, weight: $weight, style: Style::Normal, data, file: $file }
    }};
}

/// Noto Sans SC, Simplified Chinese, Regular and Bold.
#[cfg(feature = "sc")]
pub const NOTO_SANS_SC: &[Face] = &[
    face!("Noto Sans SC", 400, "NotoSansSC-Regular.woff2"),
    face!("Noto Sans SC", 700, "NotoSansSC-Bold.woff2"),
];

/// Noto Sans JP, Japanese, Regular and Bold.
#[cfg(feature = "jp")]
pub const NOTO_SANS_JP: &[Face] = &[
    face!("Noto Sans JP", 400, "NotoSansJP-Regular.woff2"),
    face!("Noto Sans JP", 700, "NotoSansJP-Bold.woff2"),
];

/// Every face compiled into this build (the enabled features).
pub fn all() -> Vec<&'static Face> {
    let mut out: Vec<&'static Face> = Vec::new();
    #[cfg(feature = "sc")]
    out.extend(NOTO_SANS_SC);
    #[cfg(feature = "jp")]
    out.extend(NOTO_SANS_JP);
    out
}

/// Whether text with `c` needs these faces: Han characters (CJK ideographs and their
/// extensions and compatibility forms), kana, bopomofo, CJK punctuation and symbols, and
/// full-width forms. For `FontCollection::add_on_demand`, so a window never showing Chinese
/// or Japanese never unpacks them.
pub fn needs(c: char) -> bool {
    matches!(
        u32::from(c),
        0x2E80..=0x2FDF // radicals
            | 0x2FF0..=0x31FF // description characters, CJK punctuation, kana, bopomofo, kanbun
            | 0x3200..=0x33FF // enclosed letters and months, compatibility
            | 0x3400..=0x4DBF // extension A
            | 0x4E00..=0x9FFF // unified ideographs
            | 0xF900..=0xFAFF // compatibility ideographs
            | 0xFE30..=0xFE4F // compatibility forms
            | 0xFF00..=0xFFEF // half-width and full-width forms
            | 0x1B000..=0x1B16F // kana supplement and extension
            | 0x20000..=0x3FFFF // extensions B and later
    )
}

/// What each face is a fallback for: Han characters by language (Chinese text in the
/// Chinese face, Japanese in the Japanese one, each falling back to the other for the
/// characters it lacks), and kana in the Japanese face. Han text in no particular language
/// takes the Chinese face first, as Unicode's own default does.
pub fn script_fallbacks() -> Vec<ScriptFallback> {
    let sc = cfg!(feature = "sc").then_some("Noto Sans SC");
    let jp = cfg!(feature = "jp").then_some("Noto Sans JP");
    let mut out = Vec::new();
    let mut add = |script: &'static str, locale: Option<&'static str>, order: [Option<&'static str>; 2]| {
        for family in order.into_iter().flatten() {
            out.push(ScriptFallback { script, locale, family });
        }
    };
    add("Hani", None, [sc, jp]);
    add("Hani", Some("zh-CN"), [sc, jp]);
    add("Hani", Some("ja"), [jp, sc]);
    for kana in ["Hira", "Kana"] {
        add(kana, None, [jp, sc]);
    }
    // Bopomofo marks and CJK symbols in Chinese text.
    add("Bopo", None, [sc, jp]);
    out
}
