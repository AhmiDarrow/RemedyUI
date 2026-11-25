//! Claps, a RemedyUI widget written in Rust and compiled to WebAssembly: it counts claps
//! toward a goal and greets whoever is looking. It is the same widget as
//! `crates/remedy-ui-engine/examples/script_lua/claps.lua`, through the WebAssembly widget ABI
//! (`remedy-ui-engine::script_wasm`'s crate docs): it exports `memory`, `remedy_alloc`,
//! `remedy_describe`, `remedy_render` and `remedy_act`, and imports `remedy::fail` to report
//! a panic.
//!
//! It reads and writes the protocol's JSON by hand to stay free of dependencies; a real
//! widget would use serde_json.

use std::cell::{Cell, RefCell};
use std::fmt::Write;

#[link(wasm_import_module = "remedy")]
unsafe extern "C" {
    /// Stops the call with this UTF-8 text as the reason.
    #[link_name = "fail"]
    fn remedy_fail(ptr: *const u8, len: usize);
}

thread_local! {
    /// Where the host writes the input of the next call.
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// What the last call returned.
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// The widget's state.
    static CLAPS: Cell<u64> = const { Cell::new(0) };
}

/// Hands the host room for `len` bytes of input.
#[unsafe(no_mangle)]
pub extern "C" fn remedy_alloc(len: i32) -> i32 {
    INPUT.with_borrow_mut(|input| {
        input.clear();
        input.resize(usize::try_from(len).unwrap_or(0), 0);
        input.as_mut_ptr() as i32
    })
}

/// Returns `text` to the host as a place in memory.
fn give(text: String) -> i64 {
    OUTPUT.with_borrow_mut(|out| {
        *out = text;
        ((out.as_ptr() as u32 as u64) << 32 | out.len() as u32 as u64) as i64
    })
}

/// The input the host wrote.
fn input(len: i32) -> Json {
    INPUT.with_borrow(|input| {
        let bytes = input.get(..usize::try_from(len).unwrap_or(0)).unwrap_or_default();
        Parser { text: bytes, at: 0 }.value().unwrap_or(Json::Null)
    })
}

/// Its name and settings.
#[unsafe(no_mangle)]
pub extern "C" fn remedy_describe() -> i64 {
    // A panic reaches the host as the reason the call stopped.
    std::panic::set_hook(Box::new(|info| {
        let why = info.to_string();
        // SAFETY: the host reads `len` bytes at `ptr`, which `why` holds for the call.
        unsafe { remedy_fail(why.as_ptr(), why.len()) }
    }));
    give(
        r#"{"name":"Claps","settings":[{"key":"title","label":"Title","kind":"text","value":"Claps"},{"key":"goal","label":"Goal","kind":"number","min":1,"max":100,"value":10}]}"#
            .to_owned(),
    )
}

/// How it looks, for `{"settings": .., "host": ..}`.
#[unsafe(no_mangle)]
pub extern "C" fn remedy_render(_ptr: i32, len: i32) -> i64 {
    let input = input(len);
    let settings = input.get("settings");
    let host = input.get("host");
    let title = settings.get("title").as_str().unwrap_or("Claps").to_owned();
    let goal = settings.get("goal").as_f64().unwrap_or(10.0);
    let house = host.get("house").as_str().unwrap_or("nobody").to_owned();
    let name = host.get("name").as_str().unwrap_or("guest").to_owned();
    let claps = CLAPS.get();
    let mut out = String::from(r#"{"kind":"column","gap":8,"children":["#);
    let _ = write!(out, r#"{{"kind":"text","text":{},"style":"heading"}},"#, quote(&title));
    let _ = write!(
        out,
        r#"{{"kind":"text","text":{},"style":"body"}},"#,
        quote(&format!("{claps} claps from {house}"))
    );
    let _ = write!(out, r#"{{"kind":"progress","label":"Claps","value":{}}},"#, claps as f64 / goal);
    out.push_str(r#"{"kind":"button","label":"Clap","on":"clap"},"#);
    let _ =
        write!(out, r#"{{"kind":"text","text":{},"style":"muted"}}]}}"#, quote(&format!("Hello, {name}")));
    give(out)
}

/// Runs the pressed button's handler.
#[unsafe(no_mangle)]
pub extern "C" fn remedy_act(_ptr: i32, len: i32) {
    match input(len).as_str() {
        Some("clap") => CLAPS.set(CLAPS.get() + 1),
        other => panic!("no handler {other:?}"),
    }
}

/// `s` as a JSON string.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON value.
enum Json {
    Null,
    Bool,
    Number(f64),
    Text(String),
    List,
    Map(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> &Json {
        match self {
            Json::Map(m) => m.iter().find(|(k, _)| k == key).map_or(&Json::Null, |(_, v)| v),
            _ => &Json::Null,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Text(s) => Some(s),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// Reads the JSON the host writes (which is always well formed).
struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<u8> {
        while let Some(b' ' | b'\n' | b'\r' | b'\t') = self.text.get(self.at) {
            self.at += 1;
        }
        self.text.get(self.at).copied()
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        (self.peek()? == b).then(|| self.at += 1)
    }

    fn value(&mut self) -> Option<Json> {
        match self.peek()? {
            b'{' => {
                self.at += 1;
                let mut m = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(Json::Map(m));
                }
                loop {
                    let Json::Text(k) = self.value()? else { return None };
                    self.eat(b':')?;
                    m.push((k, self.value()?));
                    if self.eat(b',').is_none() {
                        self.eat(b'}')?;
                        return Some(Json::Map(m));
                    }
                }
            }
            b'[' => {
                self.at += 1;
                if self.eat(b']').is_some() {
                    return Some(Json::List);
                }
                loop {
                    self.value()?;
                    if self.eat(b',').is_none() {
                        self.eat(b']')?;
                        return Some(Json::List);
                    }
                }
            }
            b'"' => {
                self.at += 1;
                let mut s = Vec::new();
                loop {
                    let b = *self.text.get(self.at)?;
                    self.at += 1;
                    match b {
                        b'"' => return Some(Json::Text(String::from_utf8(s).ok()?)),
                        b'\\' => {
                            let e = *self.text.get(self.at)?;
                            self.at += 1;
                            match e {
                                b'n' => s.push(b'\n'),
                                b't' => s.push(b'\t'),
                                b'r' => s.push(b'\r'),
                                b'b' => s.push(8),
                                b'f' => s.push(12),
                                b'u' => {
                                    let hex =
                                        std::str::from_utf8(self.text.get(self.at..self.at + 4)?).ok()?;
                                    self.at += 4;
                                    let c = char::from_u32(u32::from_str_radix(hex, 16).ok()?)
                                        .unwrap_or('\u{fffd}');
                                    s.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                                }
                                other => s.push(other),
                            }
                        }
                        other => s.push(other),
                    }
                }
            }
            b't' | b'f' => {
                let word: &[u8] = if self.text.get(self.at) == Some(&b't') { b"true" } else { b"false" };
                self.at += word.len();
                Some(Json::Bool)
            }
            b'n' => {
                self.at += 4;
                Some(Json::Null)
            }
            _ => {
                let start = self.at;
                while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') = self.text.get(self.at) {
                    self.at += 1;
                }
                std::str::from_utf8(self.text.get(start..self.at)?).ok()?.parse().ok().map(Json::Number)
            }
        }
    }
}
