//! A strict parser for the words files' one shape: a JSON object whose values are all
//! strings. No numbers, arrays, nesting or duplicate keys; the escapes are JSON's.

use std::fmt;

/// Why a words file did not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseError {
    /// The byte offset where parsing stopped.
    pub offset: usize,
    /// What was wrong there.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn fail<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError { offset: self.at, message: message.into() })
    }

    fn peek(&self) -> Option<char> {
        self.text.get(self.at..).and_then(|s| s.chars().next())
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, want: char) -> Result<(), ParseError> {
        self.skip_space();
        match self.peek() {
            Some(c) if c == want => {
                self.at += c.len_utf8();
                Ok(())
            }
            Some(c) => self.fail(format!("expected `{want}`, found `{c}`")),
            None => self.fail(format!("expected `{want}`, found the end")),
        }
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut v = 0;
        for _ in 0..4 {
            match self.bump().and_then(|c| c.to_digit(16)) {
                Some(d) => v = v * 16 + d,
                None => return self.fail("expected four hex digits after \\u"),
            }
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return self.fail("unterminated string"),
                Some('"') => return Ok(out),
                Some('\\') => {
                    let c = match self.bump() {
                        Some('"') => '"',
                        Some('\\') => '\\',
                        Some('/') => '/',
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('b') => '\u{8}',
                        Some('f') => '\u{c}',
                        Some('u') => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self.bump() != Some('\\') || self.bump() != Some('u') {
                                    return self
                                        .fail("a high surrogate must be followed by \\u and a low one");
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.fail("expected a low surrogate");
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            match char::from_u32(code) {
                                Some(c) => c,
                                None => return self.fail("not a character"),
                            }
                        }
                        Some(c) => return self.fail(format!("unknown escape \\{c}")),
                        None => return self.fail("unterminated escape"),
                    };
                    out.push(c);
                }
                Some(c) if (c as u32) < 0x20 => return self.fail("control character in a string"),
                Some(c) => out.push(c),
            }
        }
    }
}

/// Parses a words file: a JSON object mapping strings to strings, in file order. Anything
/// else (another value type, a duplicate key, text after the object) is an error.
///
/// ```
/// let pairs = remedy_ui_words::parse(r#"{"Close": "Fermer", "Line\nbreak": "é"}"#);
/// assert_eq!(pairs.map(|p| p.len()), Ok(2));
/// assert!(remedy_ui_words::parse(r#"{"a": 1}"#).is_err());
/// ```
pub fn parse(text: &str) -> Result<Vec<(String, String)>, ParseError> {
    let mut r = Reader { text, at: 0 };
    // A byte-order mark is tolerated, as editors add one.
    if r.peek() == Some('\u{feff}') {
        r.at += '\u{feff}'.len_utf8();
    }
    r.expect('{')?;
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    r.skip_space();
    if r.peek() == Some('}') {
        r.at += 1;
    } else {
        loop {
            r.skip_space();
            let start = r.at;
            let key = r.string()?;
            r.expect(':')?;
            r.skip_space();
            if r.peek() != Some('"') {
                return r.fail("every value must be a string");
            }
            let value = r.string()?;
            if !seen.insert(key.clone()) {
                return Err(ParseError { offset: start, message: format!("duplicate key {key:?}") });
            }
            pairs.push((key, value));
            r.skip_space();
            match r.bump() {
                Some(',') => {}
                Some('}') => break,
                Some(c) => return r.fail(format!("expected `,` or `}}`, found `{c}`")),
                None => return r.fail("unterminated object"),
            }
        }
    }
    r.skip_space();
    if r.at < text.len() {
        return r.fail("text after the object");
    }
    Ok(pairs)
}

/// `pairs` as a words file: one key a line, in the order given, with only the escapes JSON
/// needs (other characters as they are).
#[cfg(test)]
pub(crate) fn write(pairs: &[(String, String)]) -> String {
    fn quoted(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    if pairs.is_empty() {
        return "{}\n".to_owned();
    }
    let mut out = String::from("{\n");
    for (i, (k, v)) in pairs.iter().enumerate() {
        out.push_str("  ");
        quoted(k, &mut out);
        out.push_str(": ");
        quoted(v, &mut out);
        out.push_str(if i + 1 == pairs.len() { "\n" } else { ",\n" });
    }
    out.push_str("}\n");
    out
}
