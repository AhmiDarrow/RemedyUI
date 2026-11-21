//! Arabic, Hebrew and Chinese paragraphs lay out to match reference shapes recorded from
//! implementations the engine does not use: HarfBuzz for glyph ids, clusters, advances and
//! mark offsets, GNU FriBidi for the levels that order the runs, and ICU4C for where lines may
//! break.
//!
//! The fixtures (`fixtures/shaping`) are Noto Sans Arabic, Noto Sans Hebrew and Noto Sans SC cut
//! down to the test paragraphs (SIL Open Font License 1.1, `fixtures/shaping/OFL.txt`), and
//! `references.txt`, both written by `tools/shape_references.py`. Text is laid out at one pixel
//! per font unit, so the references' integer units are the engine's pixels.

use remedy_ui_text::{Direction, FontCollection, ParagraphOptions, RichText, TextEngine, TextStyle};

const ARABIC: &[u8] = include_bytes!("fixtures/shaping/NotoSansArabic-Subset.ttf");
const HEBREW: &[u8] = include_bytes!("fixtures/shaping/NotoSansHebrew-Subset.ttf");
const CJK: &[u8] = include_bytes!("fixtures/shaping/NotoSansSC-Subset.ttf");
const REFERENCES: &str = include_str!("fixtures/shaping/references.txt");

/// One glyph as hb-shape prints it.
#[derive(Debug, PartialEq)]
struct RefGlyph {
    id: u32,
    cluster: usize,
    dx: f32,
    dy: f32,
    advance: f32,
}

#[derive(Debug)]
struct RefRun {
    range: std::ops::Range<usize>,
    rtl: bool,
    glyphs: Vec<RefGlyph>,
}

#[derive(Debug, Default)]
struct Case {
    name: String,
    font: String,
    lang: String,
    dir: String,
    upem: f32,
    text: String,
    width: Option<f32>,
    runs: Vec<RefRun>,
    lines: Vec<std::ops::Range<usize>>,
}

type Res<T> = Result<T, String>;

fn num<T: std::str::FromStr>(s: Option<&str>, what: &str) -> Res<T> {
    s.and_then(|s| s.parse().ok()).ok_or_else(|| format!("references.txt: bad {what}: {s:?}"))
}

/// `[gid=cluster@dx,dy+advance|...]`
fn glyphs(s: &str) -> Res<Vec<RefGlyph>> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split('|')
        .map(|g| {
            let (id, rest) = g.split_once('=').unwrap_or_default();
            let (cluster, rest) = rest.split_once('@').unwrap_or_default();
            let (offsets, advance) = rest.split_once('+').unwrap_or_default();
            let (dx, dy) = offsets.split_once(',').unwrap_or_default();
            Ok(RefGlyph {
                id: num(Some(id), "glyph id")?,
                cluster: num(Some(cluster), "cluster")?,
                dx: num(Some(dx), "x offset")?,
                dy: num(Some(dy), "y offset")?,
                advance: num(Some(advance), "advance")?,
            })
        })
        .collect()
}

fn cases() -> Res<Vec<Case>> {
    let mut out: Vec<Case> = Vec::new();
    for line in REFERENCES.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key == "case" {
            out.push(Case { name: value.to_owned(), ..Case::default() });
            continue;
        }
        let c = out.last_mut().ok_or_else(|| format!("references.txt: {line:?} before any case"))?;
        match key {
            "font" => c.font = value.to_owned(),
            "lang" => c.lang = value.to_owned(),
            "dir" => c.dir = value.to_owned(),
            "upem" => c.upem = num(Some(value), "upem")?,
            "text" => c.text = value.to_owned(),
            "width" => c.width = Some(num(Some(value), "width")?),
            "run" => {
                let mut parts = value.splitn(4, ' ');
                let start = num(parts.next(), "run start")?;
                let end = num(parts.next(), "run end")?;
                let rtl = parts.next() == Some("rtl");
                let glyphs = glyphs(parts.next().unwrap_or_default())?;
                c.runs.push(RefRun { range: start..end, rtl, glyphs });
            }
            "line" => {
                let mut parts = value.split(' ');
                c.lines.push(num(parts.next(), "line start")?..num(parts.next(), "line end")?);
            }
            _ => return Err(format!("references.txt: unknown line {line:?}")),
        }
    }
    Ok(out)
}

fn font(name: &str) -> Res<&'static [u8]> {
    match name {
        "NotoSansArabic-Subset.ttf" => Ok(ARABIC),
        "NotoSansHebrew-Subset.ttf" => Ok(HEBREW),
        "NotoSansSC-Subset.ttf" => Ok(CJK),
        other => Err(format!("no fixture font {other}")),
    }
}

/// The case laid out with its fixture face only, one pixel per font unit.
fn layout(c: &Case) -> Res<remedy_ui_text::Paragraph> {
    let mut fonts = FontCollection::new();
    let families = fonts.add_static(font(&c.font)?).map_err(|e| format!("{}: {e:?}", c.font))?;
    let mut e = TextEngine::new(fonts);
    assert!(e.set_locale(&c.lang), "{}: locale {}", c.name, c.lang);
    let mut style = TextStyle::default();
    style.families = families;
    style.size = c.upem;
    let mut o = ParagraphOptions::default().snap(false);
    o.direction = match c.dir.as_str() {
        "ltr" => Direction::Ltr,
        "rtl" => Direction::Rtl,
        _ => Direction::Auto,
    };
    if let Some(w) = c.width {
        o = o.max_width(w);
    }
    Ok(e.layout(&RichText::plain(&c.text), &style, o))
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// Runs in visual order, each one's glyphs: ids, clusters, advances, and the offsets marks
/// sit at from their pen position.
fn check_shapes(c: &Case) -> Res<()> {
    let p = layout(c)?;
    let lines = p.lines();
    assert_eq!(lines.len(), 1, "{}: {lines:?}", c.name);
    let baseline = lines.first().map_or(0.0, |l| l.baseline);
    let runs = p.glyph_runs();
    let got: Vec<(std::ops::Range<usize>, bool)> = runs.iter().map(|r| (r.range.clone(), r.rtl)).collect();
    let want: Vec<(std::ops::Range<usize>, bool)> = c.runs.iter().map(|r| (r.range.clone(), r.rtl)).collect();
    assert_eq!(got, want, "{}: runs (range, rtl) in visual order", c.name);
    // The runs abut from the left edge; every glyph's pen position follows the advances.
    let mut pen = 0.0;
    for (run, reference) in runs.iter().zip(&c.runs) {
        let shaped: Vec<RefGlyph> = run
            .glyphs
            .iter()
            .zip(&run.advances)
            .zip(&run.clusters)
            .map(|((g, &advance), &cluster)| {
                let dx = g.x - pen;
                pen += advance;
                RefGlyph { id: g.id, cluster, dx, dy: baseline - g.y, advance }
            })
            .collect();
        assert_eq!(shaped.len(), run.glyphs.len(), "{}: advances and clusters for every glyph", c.name);
        let same = shaped.len() == reference.glyphs.len()
            && shaped.iter().zip(&reference.glyphs).all(|(a, b)| {
                a.id == b.id
                    && a.cluster == b.cluster
                    && near(a.advance, b.advance)
                    && near(a.dx, b.dx)
                    && near(a.dy, b.dy)
            });
        assert!(
            same,
            "{} run {:?} {:?}:\n  engine    {shaped:?}\n  reference {:?}",
            c.name,
            reference.range,
            c.text.get(reference.range.clone()),
            reference.glyphs
        );
    }
    Ok(())
}

fn case(name: &str) -> Res<Case> {
    cases()?.into_iter().find(|c| c.name == name).ok_or_else(|| format!("no case {name} in references.txt"))
}

#[test]
fn arabic_joins_ligates_and_places_marks_like_harfbuzz() -> Res<()> {
    // Joining forms, lam-alef, the tanween over its alef, and the Arabic-Indic digit as its
    // own left-to-right run inside the right-to-left paragraph.
    check_shapes(&case("arabic")?)
}

#[test]
fn hebrew_points_and_numbers_match_harfbuzz_and_fribidi() -> Res<()> {
    // Points stacked on their letters share the letter's cluster; the number and the Latin
    // name read left to right in their places; the full stop ends up at the far left.
    check_shapes(&case("hebrew")?)
}

#[test]
fn chinese_ideographs_and_full_width_punctuation_match_harfbuzz() -> Res<()> {
    check_shapes(&case("cjk")?)
}

#[test]
fn mixed_direction_runs_are_ordered_like_fribidi() -> Res<()> {
    // In a left-to-right paragraph the Hebrew phrase reverses and its number stays to its
    // right; in a right-to-left one the English phrase and the number read forwards.
    check_shapes(&case("bidi-ltr")?)?;
    check_shapes(&case("bidi-rtl")?)
}

fn check_lines(c: &Case) -> Res<()> {
    let p = layout(c)?;
    let got: Vec<std::ops::Range<usize>> = p.lines().into_iter().map(|l| l.range).collect();
    // The references fill each line greedily from HarfBuzz's advances up to the last break
    // opportunity ICU allows within the width, trailing spaces hanging past it.
    assert_eq!(got, c.lines, "{}: lines at width {:?}", c.name, c.width);
    Ok(())
}

#[test]
fn arabic_wraps_at_the_spaces_icu_allows() -> Res<()> {
    check_lines(&case("arabic-wrap")?)
}

#[test]
fn chinese_wraps_between_ideographs_but_never_before_a_full_stop() -> Res<()> {
    // The width would fit the ideograph before the full stop; the stop may not start a line,
    // so that ideograph moves down with it.
    let c = case("cjk-wrap")?;
    check_lines(&c)?;
    let starts: Vec<char> = c.lines.iter().filter_map(|l| c.text.get(l.clone())?.chars().next()).collect();
    assert!(!starts.contains(&'。'), "{starts:?}");
    Ok(())
}

#[test]
fn every_reference_case_is_checked() -> Res<()> {
    let names: Vec<String> = cases()?.into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["arabic", "hebrew", "cjk", "bidi-ltr", "bidi-rtl", "arabic-wrap", "cjk-wrap"]);
    Ok(())
}
