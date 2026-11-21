//! Subtitle and caption files: WebVTT, SubRip (SRT) and the basics of ASS/SSA (dialogue
//! lines with their override tags removed). Cues are kept as plain text, line breaks kept.

use std::time::Duration;

/// One caption: text shown from `start` until `end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    /// When it shows.
    pub start: Duration,
    /// When it goes.
    pub end: Duration,
    /// What it says; lines separated by `\n`.
    pub text: String,
}

/// A subtitle file's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubtitleFormat {
    /// WebVTT (`.vtt`).
    WebVtt,
    /// SubRip (`.srt`).
    Srt,
    /// Advanced SubStation Alpha or SubStation Alpha (`.ass`, `.ssa`).
    Ass,
}

impl SubtitleFormat {
    /// The format a file name's extension says, if it is one.
    pub fn from_extension(ext: &str) -> Option<SubtitleFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "vtt" => Some(SubtitleFormat::WebVtt),
            "srt" => Some(SubtitleFormat::Srt),
            "ass" | "ssa" => Some(SubtitleFormat::Ass),
            _ => None,
        }
    }

    /// The format of `text`, from how it starts.
    pub fn sniff(text: &str) -> Option<SubtitleFormat> {
        let t = text.trim_start_matches('\u{feff}').trim_start();
        if t.starts_with("WEBVTT") {
            Some(SubtitleFormat::WebVtt)
        } else if t.starts_with("[Script Info]") || t.contains("\nDialogue:") || t.starts_with("Dialogue:") {
            Some(SubtitleFormat::Ass)
        } else if t.lines().take(3).any(|l| l.contains("-->")) {
            Some(SubtitleFormat::Srt)
        } else {
            None
        }
    }
}

/// A caption track: where it came from, what to call it and its cues in time order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleTrack {
    /// What to call it in the captions menu ("English").
    pub label: String,
    /// Its language (BCP 47), when known.
    pub language: Option<String>,
    /// The cues, sorted by start.
    pub cues: Vec<Cue>,
}

impl SubtitleTrack {
    /// The text showing at `t`: every cue covering it, joined by line breaks, or `None`.
    pub fn text_at(&self, t: Duration) -> Option<String> {
        // Cues are sorted by start: everything after the first cue starting past `t` is out.
        let end = self.cues.partition_point(|c| c.start <= t);
        let lines: Vec<&str> = self
            .cues
            .get(..end)
            .unwrap_or_default()
            .iter()
            .filter(|c| t < c.end)
            .map(|c| c.text.as_str())
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }
}

/// Reads a subtitle file. Cues that cannot be read are skipped; an empty result means none
/// could be.
pub fn parse_subtitles(text: &str, format: SubtitleFormat) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = match format {
        SubtitleFormat::WebVtt | SubtitleFormat::Srt => parse_blocks(&text, format),
        SubtitleFormat::Ass => parse_ass(&text),
    };
    cues.sort_by_key(|c| c.start);
    cues
}

/// `hh:mm:ss.mmm`, `mm:ss.mmm` (WebVTT) or `hh:mm:ss,mmm` (SRT); ASS uses `h:mm:ss.cc`.
fn parse_time(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (whole, frac) = match s.rfind(['.', ',']) {
        Some(i) => (s.get(..i)?, s.get(i + 1..)?),
        None => (s, ""),
    };
    let mut secs: u64 = 0;
    let parts: Vec<&str> = whole.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for p in &parts {
        let n: u64 = p.trim().parse().ok()?;
        secs = secs.checked_mul(60)?.checked_add(n)?;
    }
    let mut nanos: u64 = 0;
    if !frac.is_empty() {
        if !frac.bytes().all(|b| b.is_ascii_digit()) || frac.len() > 9 {
            return None;
        }
        let n: u64 = frac.parse().ok()?;
        nanos = n * 10u64.pow(9 - u32::try_from(frac.len()).ok()?);
    }
    Some(Duration::new(secs, u32::try_from(nanos).ok()?))
}

/// The start and end of a `start --> end [settings]` line.
fn parse_timing(line: &str) -> Option<(Duration, Duration)> {
    let (a, b) = line.split_once("-->")?;
    let b = b.split_whitespace().next()?;
    let (start, end) = (parse_time(a)?, parse_time(b)?);
    (end > start).then_some((start, end))
}

/// WebVTT and SRT: blank-line separated blocks, each with a timing line and text after it.
fn parse_blocks(text: &str, format: SubtitleFormat) -> Vec<Cue> {
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let first = lines.first().map(|l| l.trim_start()).unwrap_or_default();
        if format == SubtitleFormat::WebVtt
            && (first.starts_with("NOTE") || first.starts_with("STYLE") || first.starts_with("REGION"))
        {
            continue;
        }
        let Some((start, end)) = lines.get(ti).and_then(|l| parse_timing(l)) else { continue };
        let body: Vec<String> =
            lines.get(ti + 1..).unwrap_or_default().iter().map(|l| clean_markup(l)).collect();
        let text = body.join("\n").trim().to_string();
        if !text.is_empty() {
            cues.push(Cue { start, end, text });
        }
    }
    cues
}

/// Removes `<i>`, `<c.class>`, `<v Name>` and similar tags, SRT's `{\an8}` positioning, and
/// decodes the entities WebVTT allows.
pub(crate) fn clean_markup(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                for d in chars.by_ref() {
                    if d == '>' {
                        break;
                    }
                }
            }
            '{' if chars.peek() == Some(&'\\') => {
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}")
        .replace("&lrm;", "\u{200e}")
        .replace("&rlm;", "\u{200f}")
        .replace("&amp;", "&")
}

/// ASS/SSA: the `[Events]` section's `Format:` line names the fields, and each `Dialogue:`
/// line fills them, its text last (it may hold commas).
fn parse_ass(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut fields: Vec<String> =
        ["layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let mut in_events = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events && !line.starts_with("Dialogue:") {
            continue;
        }
        if let Some(f) = line.strip_prefix("Format:") {
            fields = f.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
            continue;
        }
        let Some(d) = line.strip_prefix("Dialogue:") else { continue };
        let parts: Vec<&str> = d.splitn(fields.len().max(1), ',').collect();
        let field = |name: &str| fields.iter().position(|f| f == name).and_then(|i| parts.get(i)).copied();
        let (Some(s), Some(e), Some(t)) = (field("start"), field("end"), field("text")) else { continue };
        let (Some(start), Some(end)) = (parse_time(s), parse_time(e)) else { continue };
        if end <= start {
            continue;
        }
        let text = ass_text(t);
        if !text.is_empty() {
            cues.push(Cue { start, end, text });
        }
    }
    cues
}

/// An ASS text field as plain text: `{…}` override blocks removed, `\N` and `\n` as line
/// breaks, `\h` as a no-break space.
pub(crate) fn ass_text(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                }
            }
            '\\' => match chars.peek() {
                Some('N') | Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('h') => {
                    chars.next();
                    out.push('\u{a0}');
                }
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn webvtt_cues_with_ids_settings_notes_and_tags() {
        let vtt = "WEBVTT\n\nNOTE a comment\n--> not a cue\n\nintro\n00:01.000 --> 00:02.500 align:start\n<v Alex>Hello <i>there</i></v>\n\n00:00:03.000 --> 00:00:04.000\nTwo &amp; lines\nsecond\n";
        let cues = parse_subtitles(vtt, SubtitleFormat::WebVtt);
        assert_eq!(cues.len(), 2, "{cues:?}");
        assert_eq!(cues.first(), Some(&Cue { start: ms(1000), end: ms(2500), text: "Hello there".into() }));
        assert_eq!(cues.get(1).map(|c| c.text.as_str()), Some("Two & lines\nsecond"));
        assert_eq!(SubtitleFormat::sniff(vtt), Some(SubtitleFormat::WebVtt));
    }

    #[test]
    fn srt_with_crlf_and_positioning() {
        let srt = "1\r\n00:00:00,500 --> 00:00:01,250\r\n{\\an8}Top <b>line</b>\r\n\r\n2\r\n00:00:02,000 --> 00:00:03,000\r\nNext\r\n";
        let cues = parse_subtitles(srt, SubtitleFormat::Srt);
        assert_eq!(
            cues,
            vec![
                Cue { start: ms(500), end: ms(1250), text: "Top line".into() },
                Cue { start: ms(2000), end: ms(3000), text: "Next".into() },
            ]
        );
        assert_eq!(SubtitleFormat::sniff(srt), Some(SubtitleFormat::Srt));
    }

    #[test]
    fn ass_dialogue_with_commas_and_overrides() {
        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\i1}Hi,{\\i0} you\\Nthere\n";
        let cues = parse_subtitles(ass, SubtitleFormat::Ass);
        assert_eq!(cues, vec![Cue { start: ms(1000), end: ms(2500), text: "Hi, you\nthere".into() }]);
    }

    #[test]
    fn overlapping_cues_show_together() {
        let track = SubtitleTrack {
            label: "English".into(),
            language: Some("en".into()),
            cues: vec![
                Cue { start: ms(0), end: ms(2000), text: "a".into() },
                Cue { start: ms(1000), end: ms(3000), text: "b".into() },
            ],
        };
        assert_eq!(track.text_at(ms(500)).as_deref(), Some("a"));
        assert_eq!(track.text_at(ms(1500)).as_deref(), Some("a\nb"));
        assert_eq!(track.text_at(ms(2500)).as_deref(), Some("b"));
        assert_eq!(track.text_at(ms(3000)), None);
    }

    #[test]
    fn bad_times_are_skipped() {
        assert_eq!(parse_time("1:02:03.5"), Some(Duration::new(3723, 500_000_000)));
        assert_eq!(parse_time("x:00"), None);
        assert!(parse_subtitles("00:02.000 --> 00:01.000\nbackwards", SubtitleFormat::WebVtt).is_empty());
    }
}
