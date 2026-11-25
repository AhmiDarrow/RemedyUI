//! RemedyUI's widgets: the core set, navigation, forms, data, music, stage, board and extra widgets, live visuals, themes, customization, markup, document views, developer tools, automation, spell checking and the update banner.
//!
//! Each part is a module behind a Cargo feature of the same name.

#[cfg(feature = "base")]
pub mod base;
#[cfg(feature = "nav")]
pub mod nav;
#[cfg(feature = "forms")]
pub mod forms;
#[cfg(feature = "data")]
pub mod data;
#[cfg(feature = "extra")]
pub mod extra;
#[cfg(feature = "stage")]
pub mod stage;
#[cfg(feature = "music")]
pub mod music;
#[cfg(feature = "board")]
pub mod board;
#[cfg(feature = "viz")]
pub mod viz;
#[cfg(feature = "themes")]
pub mod themes;
#[cfg(feature = "customize")]
pub mod customize;
#[cfg(feature = "rui")]
pub mod rui;
#[cfg(feature = "doc-rich")]
pub mod doc_rich;
#[cfg(feature = "doc-code")]
pub mod doc_code;
#[cfg(feature = "doc-pages")]
pub mod doc_pages;
#[cfg(feature = "devtools")]
pub mod devtools;
#[cfg(feature = "automation")]
pub mod automation;
#[cfg(feature = "spell")]
pub mod spell;
#[cfg(feature = "update")]
pub mod update;
