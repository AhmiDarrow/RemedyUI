//! Subtitle and caption files: WebVTT, SubRip (SRT) and the basics of ASS/SSA (dialogue
//! lines with their override tags removed). Cues are kept as plain text, line breaks kept.

use std::time::Duration;

use crate::host::MediaError;

/// The largest subtitle file read (a feature film's SRT is about 100 KB, a styled ASS a few MB).
pub(crate) const MAX_SUBTITLE_BYTES: u64 = 16 << 20;

/// Why the cues inside a media file or stream were refused: their text passed
/// [`MAX_SUBTITLE_BYTES`], as a subtitle file may not.
pub(crate) const CUES_TOO_BIG: &str = "the subtitles inside this media are over 16 MB";

/// The longest single cue a stream may hand out: a cue is a line or two, so one this long is
/// not captions.
pub(crate) const MAX_STREAMED_CUE: usize = 1 << 20;

/// Why a stream was refused: one of its cues was over [`MAX_STREAMED_CUE`].
pub(crate) const CUE_TOO_BIG: &str = "a subtitle cue inside this stream is over 1 MB";

/// Adds a stream's `new` cues to the ones it handed out before (`cues`, in start order). A
/// cue already held (the stream read again after a seek back) is not added twice, and past
/// [`MAX_SUBTITLE_BYTES`] the earliest go, so a stream that runs for days keeps a window of
/// its latest captions.
pub(crate) fn add_streamed_cues(cues: &mut Vec<Cue>, new: impl IntoIterator<Item = Cue>) {
    let size = |c: &Cue| (c.text.len() + std::mem::size_of::<Cue>()) as u64;
    for cue in new {
        let at = cues.partition_point(|c| c.start <= cue.start);
        if cues
            .get(..at)
            .unwrap_or_default()
            .iter()
            .rev()
            .take_while(|c| c.start == cue.start)
            .any(|c| *c == cue)
        {
            continue;
        }
        cues.insert(at, cue);
    }
    let mut total: u64 = cues.iter().map(size).sum();
    let mut drop = 0;
    while total > MAX_SUBTITLE_BYTES && drop < cues.len() {
        total -= cues.get(drop).map_or(0, size);
        drop += 1;
    }
    cues.drain(..drop);
}

/// Counts `text` against the cues read so far; true once they pass [`MAX_SUBTITLE_BYTES`].
pub(crate) fn over_cue_budget(total: &mut u64, text: &str) -> bool {
    *total = total.saturating_add(text.len() as u64);
    *total > MAX_SUBTITLE_BYTES
}

/// Reads the subtitle file at `path` as text, refusing one over [`MAX_SUBTITLE_BYTES`].
pub(crate) fn read_subtitle_file(path: &std::path::Path) -> Result<String, MediaError> {
    use std::io::Read;
    let failed = |e: std::io::Error| MediaError::NotFound(format!("{}: {e}", path.display()));
    let mut text = String::new();
    // Read one byte past the limit, so a file growing while read is caught too.
    std::fs::File::open(path)
        .map_err(failed)?
        .take(MAX_SUBTITLE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(failed)?;
    if text.len() as u64 > MAX_SUBTITLE_BYTES {
        return Err(MediaError::Format(format!(
            "{} is over {} MB, too big for a subtitle file",
            path.display(),
            MAX_SUBTITLE_BYTES >> 20
        )));
    }
    Ok(text)
}

/// For each cue, the latest end of it and every cue before it: never falls, so
/// [`SubtitleTrack::text_at_with`] can search it.
pub(crate) fn cue_reach(cues: &[Cue]) -> Vec<Duration> {
    cues.iter()
        .scan(Duration::ZERO, |latest, c| {
            *latest = (*latest).max(c.end);
            Some(*latest)
        })
        .collect()
}

/// One caption: text shown from `start` until `end`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cue {
    /// When it shows.
    pub start: Duration,
    /// When it goes.
    pub end: Duration,
    /// What it says; lines separated by `\n`.
    pub text: String,
    /// Where it goes; the default is centred lines at the foot of the picture.
    pub settings: CueSettings,
}

/// Where a cue goes, in WebVTT's cue settings (`line`, `position`, `size`, `align`,
/// `vertical`); SRT's and ASS's `{\anN}` and ASS's margins are read into the same terms.
/// Percentages are of the picture (WebVTT's video viewport), 0 to 100.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CueSettings {
    /// Vertical text: lines running down, stacked right to left or left to right.
    pub vertical: Option<Vertical>,
    /// Which line: `None` sits at the foot (`line:auto`).
    pub line: Option<CueLine>,
    /// Which part of the cue a percentage `line` places (`line:50%,center`); `None` is its
    /// start (its top, in horizontal text).
    pub line_align: Option<CueAnchor>,
    /// Where the box's anchor is across, in percent; `None` follows `align`.
    pub position: Option<f32>,
    /// Which part of the box sits at `position`; `None` follows `align`.
    pub position_align: Option<CueAnchor>,
    /// The box's width in percent; `None` is 100.
    pub size: Option<f32>,
    /// How its lines line up in the box.
    pub align: CueAlign,
}

// The percentages are finite (parsing refuses others), so equality is total.
impl Eq for CueSettings {}

/// A cue's `line`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CueLine {
    /// From the top, in percent: 0 is the top, 100 the foot.
    Percent(f32),
    /// Lines of text: 0 is the top line, -1 the foot's, -2 the one above.
    Number(i32),
}

/// A cue's `align`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CueAlign {
    /// The start of the line (the left, in left-to-right text).
    Start,
    /// Centred.
    #[default]
    Center,
    /// The end of the line.
    End,
    /// The left.
    Left,
    /// The right.
    Right,
}

/// Which part of a cue's box `position` places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CueAnchor {
    /// Its left edge (`line-left`).
    Start,
    /// Its middle.
    Center,
    /// Its right edge (`line-right`).
    End,
}

/// A cue's `vertical`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Vertical {
    /// `rl`: lines run down, from right to left.
    Rl,
    /// `lr`: lines run down, from left to right.
    Lr,
}

impl CueSettings {
    /// Reads a WebVTT timing line's settings (`line:10% align:start`); unknown or bad ones
    /// are left out, as the WebVTT parser does.
    pub fn parse_webvtt(s: &str) -> CueSettings {
        let mut out = CueSettings::default();
        for (k, v) in s.split_whitespace().filter_map(|t| t.split_once(':')) {
            // `line:50%,center`, `position:10%,line-left`: the part after the comma.
            let (v, after) = v.split_once(',').unwrap_or((v, ""));
            match k {
                "vertical" => match v {
                    "rl" => out.vertical = Some(Vertical::Rl),
                    "lr" => out.vertical = Some(Vertical::Lr),
                    _ => {}
                },
                "line" => {
                    match (percent(v), v.parse()) {
                        (Some(p), _) => out.line = Some(CueLine::Percent(p)),
                        (None, Ok(n)) => out.line = Some(CueLine::Number(n)),
                        _ => continue,
                    }
                    out.line_align = match after {
                        "start" => Some(CueAnchor::Start),
                        "center" => Some(CueAnchor::Center),
                        "end" => Some(CueAnchor::End),
                        _ => None,
                    };
                }
                "position" => {
                    if let Some(p) = percent(v) {
                        out.position = Some(p);
                        out.position_align = match after {
                            "line-left" => Some(CueAnchor::Start),
                            "center" => Some(CueAnchor::Center),
                            "line-right" => Some(CueAnchor::End),
                            _ => None,
                        };
                    }
                }
                "size" => out.size = percent(v).or(out.size),
                "align" => match v {
                    "start" => out.align = CueAlign::Start,
                    "center" | "middle" => out.align = CueAlign::Center,
                    "end" => out.align = CueAlign::End,
                    "left" => out.align = CueAlign::Left,
                    "right" => out.align = CueAlign::Right,
                    _ => {}
                },
                _ => {}
            }
        }
        out
    }

    /// ASS's numpad alignment (`\an1`…`\an9`: 1 bottom left, 5 the middle, 9 top right),
    /// with the left, right and vertical margins in the script's pixels and its size
    /// (`PlayResX` by `PlayResY`): the vertical margin above a bottom line and below a top
    /// one, the side margins bounding the box. Bottom-centre with no margins is the default.
    pub fn numpad(an: u8, margins: (f32, f32, f32), res: (f32, f32)) -> CueSettings {
        if !(1..=9).contains(&an) || (an == 2 && margins == (0.0, 0.0, 0.0)) {
            return CueSettings::default();
        }
        let (h, v) = ((an - 1) % 3, (an - 1) / 3);
        let pc = |px: f32, of: f32| if of > 0.0 { (px / of * 100.0).clamp(0.0, 100.0) } else { 0.0 };
        let (l, r, mv) = (pc(margins.0, res.0), pc(margins.1, res.0), pc(margins.2, res.1));
        CueSettings {
            line: Some(CueLine::Percent(match v {
                0 => 100.0 - mv,
                1 => 50.0,
                _ => mv,
            })),
            line_align: Some(match v {
                0 => CueAnchor::End,
                1 => CueAnchor::Center,
                _ => CueAnchor::Start,
            }),
            position: Some(l),
            position_align: Some(CueAnchor::Start),
            size: Some((100.0 - l - r).max(0.0)),
            align: match h {
                0 => CueAlign::Left,
                1 => CueAlign::Center,
                _ => CueAlign::Right,
            },
            vertical: None,
        }
    }

    /// The box across, in percent of the area: its left edge and width, by WebVTT's rules
    /// (the position and its anchor from `align` when not given, the size cut to fit).
    pub fn box_across(&self) -> (f32, f32) {
        let pos = self.position.unwrap_or(match self.align {
            CueAlign::Left | CueAlign::Start => 0.0,
            CueAlign::Right | CueAlign::End => 100.0,
            CueAlign::Center => 50.0,
        });
        let anchor = self.position_align.unwrap_or(match self.align {
            CueAlign::Left | CueAlign::Start => CueAnchor::Start,
            CueAlign::Right | CueAlign::End => CueAnchor::End,
            CueAlign::Center => CueAnchor::Center,
        });
        let room = match anchor {
            CueAnchor::Start => 100.0 - pos,
            CueAnchor::End => pos,
            CueAnchor::Center => 2.0 * pos.min(100.0 - pos),
        };
        let size = self.size.unwrap_or(100.0).min(room);
        let left = match anchor {
            CueAnchor::Start => pos,
            CueAnchor::End => pos - size,
            CueAnchor::Center => pos - size / 2.0,
        };
        (left, size)
    }
}

/// `12.5%` as 12.5, if it is a percentage from 0 to 100.
fn percent(v: &str) -> Option<f32> {
    let p: f32 = v.strip_suffix('%')?.parse().ok()?;
    (p.is_finite() && (0.0..=100.0).contains(&p)).then_some(p)
}

/// The first `\anN` override in a cue's text (SRT and ASS), 1 to 9.
pub(crate) fn an_tag(t: &str) -> Option<u8> {
    let i = t.find("\\an")?;
    let d = t.get(i + 3..)?.chars().next()?.to_digit(10)?;
    u8::try_from(d).ok().filter(|d| (1..=9).contains(d))
}

/// Where an SRT cue goes: its `{\anN}` tag, read as ASS reads it.
pub(crate) fn srt_settings(t: &str) -> CueSettings {
    an_tag(t).map(|an| CueSettings::numpad(an, (0.0, 0.0, 0.0), (0.0, 0.0))).unwrap_or_default()
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
        joined(&self.covering(0, t))
    }

    /// [`text_at`](Self::text_at) in O(log n) plus the cues near `t`, with `reach` the
    /// track's [`cue_reach`].
    #[cfg(test)]
    pub(crate) fn text_at_with(&self, reach: &[Duration], t: Duration) -> Option<String> {
        joined(&self.cues_at_with(reach, t))
    }

    /// The cues covering `t`, in start order, with `reach` the track's [`cue_reach`].
    pub(crate) fn cues_at_with(&self, reach: &[Duration], t: Duration) -> Vec<&Cue> {
        // No cue before the first whose reach passes `t` can cover it.
        self.covering(reach.partition_point(|&r| r <= t), t)
    }

    /// The cues from `from` on that cover `t`.
    fn covering(&self, from: usize, t: Duration) -> Vec<&Cue> {
        // Cues are sorted by start: everything after the first cue starting past `t` is out.
        let end = self.cues.partition_point(|c| c.start <= t);
        self.cues.get(from.min(end)..end).unwrap_or_default().iter().filter(|c| t < c.end).collect()
    }
}

/// Cues' text joined by line breaks, or `None` for none.
fn joined(cues: &[&Cue]) -> Option<String> {
    let lines: Vec<&str> = cues.iter().map(|c| c.text.as_str()).collect();
    if lines.is_empty() { None } else { Some(lines.join("\n")) }
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
        let Some(timing) = lines.get(ti) else { continue };
        let Some((start, end)) = parse_timing(timing) else { continue };
        let raw = lines.get(ti + 1..).unwrap_or_default();
        let settings = match format {
            // After the end time: `00:01.000 --> 00:02.000 line:0 align:start`.
            SubtitleFormat::WebVtt => CueSettings::parse_webvtt(
                timing
                    .split_once("-->")
                    .map_or("", |(_, b)| b.trim_start().split_once(' ').map_or("", |s| s.1)),
            ),
            _ => srt_settings(&raw.join("\n")),
        };
        let body: Vec<String> = raw.iter().map(|l| clean_markup(l)).collect();
        let text = body.join("\n").trim().to_string();
        if !text.is_empty() {
            cues.push(Cue { start, end, text, settings });
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

/// An ASS/SSA script's header: its size (`PlayResX` by `PlayResY`, ASS's 384 by 288 when it
/// says none) and each style's numpad alignment and left, right and vertical margins. A
/// Matroska ASS track keeps it in its codec private data.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AssHeader {
    res: (f32, f32),
    styles: std::collections::HashMap<String, (u8, [f32; 3])>,
}

impl Default for AssHeader {
    fn default() -> Self {
        AssHeader { res: (384.0, 288.0), styles: Default::default() }
    }
}

impl AssHeader {
    /// Reads `[Script Info]`'s size and the `[V4+ Styles]` (numpad alignment) or `[V4 Styles]`
    /// (SSA's: 1-3 foot, +4 top, +8 middle) section.
    pub(crate) fn parse(text: &str) -> AssHeader {
        let mut h = AssHeader::default();
        let mut section: Option<bool> = None;
        let mut fields: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = match line.to_ascii_lowercase().as_str() {
                    "[v4+ styles]" => Some(true),
                    "[v4 styles]" => Some(false),
                    _ => None,
                };
                continue;
            }
            if let Some(v) = line.strip_prefix("PlayResX:").and_then(|v| v.trim().parse().ok()) {
                h.res.0 = v;
            }
            if let Some(v) = line.strip_prefix("PlayResY:").and_then(|v| v.trim().parse().ok()) {
                h.res.1 = v;
            }
            let Some(plus) = section else { continue };
            if let Some(f) = line.strip_prefix("Format:") {
                fields = f.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
            } else if let Some(s) = line.strip_prefix("Style:") {
                let parts: Vec<&str> = s.split(',').map(str::trim).collect();
                let field = |n: &str| fields.iter().position(|f| f == n).and_then(|i| parts.get(i)).copied();
                let num = |n: &str| field(n).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                let a: u8 = field("alignment").and_then(|v| v.parse().ok()).unwrap_or(2);
                let an = match (plus, a) {
                    (true, a) => a,
                    (false, 5..=7) => a + 2,
                    (false, 9..=11) => a - 5,
                    (false, a) => a,
                };
                let name = field("name").unwrap_or_default().to_string();
                h.styles.insert(name, (an, [num("marginl"), num("marginr"), num("marginv")]));
            }
        }
        h
    }

    /// Where a dialogue goes: its text's `\anN` over its style's alignment, and its own
    /// margins (where not 0) over its style's.
    pub(crate) fn settings(&self, style: &str, margins: [f32; 3], text: &str) -> CueSettings {
        let (style_an, style_margins) = self.styles.get(style.trim()).copied().unwrap_or((2, [0.0; 3]));
        let m = |i: usize| {
            let own = margins.get(i).copied().unwrap_or(0.0);
            if own != 0.0 { own } else { style_margins.get(i).copied().unwrap_or(0.0) }
        };
        CueSettings::numpad(an_tag(text).unwrap_or(style_an), (m(0), m(1), m(2)), self.res)
    }
}

/// ASS/SSA: the `[Events]` section's `Format:` line names the fields, and each `Dialogue:`
/// line fills them, its text last (it may hold commas).
fn parse_ass(text: &str) -> Vec<Cue> {
    let header = AssHeader::parse(text);
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
        let num = |n: &str| field(n).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
        let margins = [num("marginl"), num("marginr"), num("marginv")];
        let settings = header.settings(field("style").unwrap_or_default(), margins, t);
        let text = ass_text(t);
        if !text.is_empty() {
            cues.push(Cue { start, end, text, settings });
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
        let settings = CueSettings { align: CueAlign::Start, ..Default::default() };
        assert_eq!(
            cues.first(),
            Some(&Cue { start: ms(1000), end: ms(2500), text: "Hello there".into(), settings })
        );
        assert_eq!(cues.get(1).map(|c| c.settings), Some(CueSettings::default()));
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
                Cue {
                    start: ms(500),
                    end: ms(1250),
                    text: "Top line".into(),
                    settings: srt_settings("{\\an8}")
                },
                Cue { start: ms(2000), end: ms(3000), text: "Next".into(), ..Default::default() },
            ]
        );
        assert_eq!(SubtitleFormat::sniff(srt), Some(SubtitleFormat::Srt));
    }

    #[test]
    fn ass_dialogue_with_commas_and_overrides() {
        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\i1}Hi,{\\i0} you\\Nthere\n";
        let cues = parse_subtitles(ass, SubtitleFormat::Ass);
        assert_eq!(
            cues,
            vec![Cue { start: ms(1000), end: ms(2500), text: "Hi, you\nthere".into(), ..Default::default() }]
        );
    }

    #[test]
    fn webvtt_cue_settings() {
        let s = CueSettings::parse_webvtt("line:10% position:25%,line-left size:50% align:left vertical:rl");
        assert_eq!(s.line, Some(CueLine::Percent(10.0)));
        assert_eq!((s.position, s.position_align), (Some(25.0), Some(CueAnchor::Start)));
        assert_eq!((s.size, s.align, s.vertical), (Some(50.0), CueAlign::Left, Some(Vertical::Rl)));
        assert_eq!(s.box_across(), (25.0, 50.0));
        assert_eq!(CueSettings::parse_webvtt("line:-2 align:end").line, Some(CueLine::Number(-2)));
        let s = CueSettings::parse_webvtt("line:80%,end");
        assert_eq!((s.line, s.line_align), (Some(CueLine::Percent(80.0)), Some(CueAnchor::End)));
        assert_eq!(CueSettings::parse_webvtt("line:50%,center").line_align, Some(CueAnchor::Center));
        assert_eq!(CueSettings::parse_webvtt("line:x%,center"), CueSettings::default());
        // Bad values are left out; nothing given is the default.
        assert_eq!(
            CueSettings::parse_webvtt("line:150% size:x position:-1% bogus:1"),
            CueSettings::default()
        );
        assert_eq!(CueSettings::default().box_across(), (0.0, 100.0));
        // A centred box is cut to fit around its position; an end-aligned one ends there.
        let s = CueSettings::parse_webvtt("position:20% size:80%");
        assert_eq!(s.box_across(), (0.0, 40.0));
        assert_eq!(CueSettings::parse_webvtt("align:right size:30%").box_across(), (70.0, 30.0));
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000 line:0 position:10% align:start\nTop left\n";
        let cue = parse_subtitles(vtt, SubtitleFormat::WebVtt);
        assert_eq!(cue.first().and_then(|c| c.settings.line), Some(CueLine::Number(0)));
        assert_eq!(cue.first().and_then(|c| c.settings.position), Some(10.0));
    }

    #[test]
    fn srt_and_ass_numpad_positions() {
        let s = srt_settings("{\\an7}Top left");
        assert_eq!(
            (s.line, s.align, s.box_across()),
            (Some(CueLine::Percent(0.0)), CueAlign::Left, (0.0, 100.0))
        );
        let s = srt_settings("{\\an6}Middle right");
        assert_eq!((s.line, s.align), (Some(CueLine::Percent(50.0)), CueAlign::Right));
        assert_eq!(srt_settings("{\\an2}Bottom"), CueSettings::default());
        assert_eq!(srt_settings("{\\an0}x"), CueSettings::default());
        assert_eq!(srt_settings("plain"), CueSettings::default());
        // Styles' alignment and margins (V4+), a dialogue's own margins and `\an` over them, and
        // SSA's old alignment numbers (6 is top centre).
        let ass = "[Script Info]\nPlayResX: 1000\nPlayResY: 500\n\n[V4+ Styles]\nFormat: Name, Fontname, Alignment, MarginL, MarginR, MarginV\nStyle: Sign,Arial,9,100,50,25\nStyle: Default,Arial,2,10,10,10\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Sign,,0,0,0,,Top right\nDialogue: 0,0:00:03.00,0:00:04.00,Sign,,200,0,0,,{\\an4}Middle left\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,Foot\n";
        let cues = parse_subtitles(ass, SubtitleFormat::Ass);
        let s: Vec<CueSettings> = cues.iter().map(|c| c.settings).collect();
        let first = s.first().copied().unwrap_or_default();
        assert_eq!((first.line, first.align), (Some(CueLine::Percent(5.0)), CueAlign::Right));
        assert_eq!(first.box_across(), (10.0, 85.0));
        let second = s.get(1).copied().unwrap_or_default();
        assert_eq!(
            (second.line, second.align, second.box_across()),
            (Some(CueLine::Percent(50.0)), CueAlign::Left, (20.0, 75.0))
        );
        // A bottom line sits its MarginV (10 of 500: 2%) above the foot, by its bottom edge.
        let third = s.get(2).copied().unwrap_or_default();
        assert_eq!((third.line, third.line_align), (Some(CueLine::Percent(98.0)), Some(CueAnchor::End)));
        assert_eq!((third.align, third.box_across()), (CueAlign::Center, (1.0, 98.0)));
        let ssa = "[V4 Styles]\nFormat: Name, Alignment\nStyle: Top,6\n\n[Events]\nFormat: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: Marked=0,0:00:01.00,0:00:02.00,Top,,0,0,0,,Hi\n";
        let cue = parse_subtitles(ssa, SubtitleFormat::Ass);
        let s = cue.first().map(|c| c.settings).unwrap_or_default();
        assert_eq!((s.line, s.align), (Some(CueLine::Percent(0.0)), CueAlign::Center));
    }

    #[test]
    fn overlapping_cues_show_together() {
        let track = SubtitleTrack {
            label: "English".into(),
            language: Some("en".into()),
            cues: vec![
                Cue { start: ms(0), end: ms(2000), text: "a".into(), ..Default::default() },
                Cue { start: ms(1000), end: ms(3000), text: "b".into(), ..Default::default() },
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

    /// The indexed lookup finds what the scan does, a long cue spanning others included, and
    /// (printed) how much faster it is on a large track.
    #[test]
    fn indexed_lookup_matches_the_scan() {
        let mut cues: Vec<Cue> = (0..20_000u64)
            .map(|i| Cue {
                start: ms(i * 1000),
                end: ms(i * 1000 + 700 + i % 3 * 400),
                text: i.to_string(),
                ..Default::default()
            })
            .collect();
        cues.push(Cue {
            start: ms(5_000_500),
            end: ms(5_100_000),
            text: "long".into(),
            ..Default::default()
        });
        cues.sort_by_key(|c| c.start);
        let track = SubtitleTrack { label: "x".into(), language: None, cues };
        let reach = cue_reach(&track.cues);
        for t in (0..21_000_000u64).step_by(250).map(ms) {
            assert_eq!(track.text_at_with(&reach, t), track.text_at(t), "at {t:?}");
        }
        let late = ms(19_000_300);
        let timed = |f: &dyn Fn() -> Option<String>| {
            let started = std::time::Instant::now();
            for _ in 0..1000 {
                std::hint::black_box(f());
            }
            started.elapsed() / 1000
        };
        let scan = timed(&|| track.text_at(late));
        let indexed = timed(&|| track.text_at_with(&reach, late));
        eprintln!("20k cues, one lookup: scan {scan:?}, indexed {indexed:?}");
    }

    #[test]
    fn a_streams_cues_keep_a_window_and_come_once() {
        let cue = |s: u64, text: &str| Cue {
            start: ms(s),
            end: ms(s + 10),
            text: text.into(),
            ..Default::default()
        };
        let mut cues = Vec::new();
        add_streamed_cues(&mut cues, [cue(20, "b"), cue(10, "a")]);
        // Read again after a seek back: held already, so not added twice; a new one is.
        add_streamed_cues(&mut cues, [cue(10, "a"), cue(20, "b"), cue(20, "b2")]);
        let texts: Vec<_> = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "b2"]);
        // Past the limit, the earliest go.
        let big = "x".repeat(1 << 20);
        add_streamed_cues(&mut cues, (100..120).map(|i| cue(i * 100, &big)));
        let bytes: usize = cues.iter().map(|c| c.text.len() + std::mem::size_of::<Cue>()).sum();
        assert!(bytes as u64 <= MAX_SUBTITLE_BYTES, "{bytes}");
        assert_eq!(cues.first().map(|c| c.start), Some(ms(10_500)), "the 15 latest stay");
        assert_eq!(cues.len(), 15);
    }

    #[test]
    fn huge_subtitle_files_are_refused() {
        let path = std::env::temp_dir().join(format!("remedy-huge-{}.srt", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|f| f.set_len(MAX_SUBTITLE_BYTES + 1))
            .unwrap_or_else(|e| unreachable!("{e}"));
        let got = read_subtitle_file(&path);
        let _ = std::fs::remove_file(&path);
        assert!(matches!(&got, Err(MediaError::Format(m)) if m.contains("too big")), "{got:?}");
    }
}
