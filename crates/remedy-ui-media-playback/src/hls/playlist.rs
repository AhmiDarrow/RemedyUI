//! Reading HLS playlists (`.m3u8`, RFC 8216): a media playlist's segments, or the variant a
//! master playlist offers that this crate can play. Untrusted input: what is not understood is
//! skipped, and what would play wrongly (encrypted segments, fMP4 segments, byte ranges) is
//! refused with the reason.

use std::time::Duration;

use crate::host::MediaError;

/// One segment of a media playlist.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Segment {
    /// The absolute URL.
    pub url: String,
    /// Its media sequence number.
    pub sequence: u64,
    /// How long it lasts (`#EXTINF`).
    pub duration: Duration,
    /// Where it starts, from the first segment the playlist listed.
    pub start: Duration,
    /// A `#EXT-X-DISCONTINUITY` comes before it: its clock is not the one before it.
    pub discontinuity: bool,
}

/// A media playlist.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MediaPlaylist {
    /// `#EXT-X-TARGETDURATION`: no segment lasts longer.
    pub target: Duration,
    pub segments: Vec<Segment>,
    /// No segments will be added (`#EXT-X-ENDLIST`).
    pub ended: bool,
}

/// What a playlist is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Playlist {
    /// A list of segments.
    Media(MediaPlaylist),
    /// A list of variants: the absolute URL of the one chosen.
    Master(String),
}

fn unsupported(why: &str) -> MediaError {
    MediaError::Unsupported(format!("HLS {why}"))
}

/// Reads the playlist `text` fetched from `url`, which relative URIs in it are resolved
/// against.
pub(crate) fn parse(text: &str, url: &str) -> Result<Playlist, MediaError> {
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if !text.starts_with("#EXTM3U") {
        return Err(MediaError::Format("an HLS playlist starts with #EXTM3U".into()));
    }
    let mut target = Duration::ZERO;
    let mut sequence = 0u64;
    let mut segments = Vec::new();
    let mut ended = false;
    let mut duration: Option<Duration> = None;
    let mut discontinuity = false;
    let mut start = Duration::ZERO;
    // Master playlists: the best variant so far (bandwidth, URL), and the one being read.
    let mut best: Option<(u64, String)> = None;
    let mut variant: Option<Option<u64>> = None;
    let mut saw_variant = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(tag) = line.strip_prefix('#') {
            let (name, value) = tag.split_once(':').unwrap_or((tag, ""));
            match name {
                "EXT-X-TARGETDURATION" => {
                    target = Duration::from_secs(value.trim().parse().unwrap_or(0));
                }
                "EXT-X-MEDIA-SEQUENCE" => sequence = value.trim().parse().unwrap_or(0),
                "EXTINF" => {
                    let secs = value.split(',').next().unwrap_or("").trim();
                    let secs: f64 = secs.parse().unwrap_or(0.0);
                    duration = Some(Duration::try_from_secs_f64(secs).unwrap_or(Duration::ZERO));
                }
                "EXT-X-ENDLIST" => ended = true,
                "EXT-X-DISCONTINUITY" => discontinuity = true,
                "EXT-X-KEY" => {
                    let method = attribute(value, "METHOD").unwrap_or_default();
                    if method != "NONE" {
                        return Err(unsupported(&format!("segments are encrypted ({method})")));
                    }
                }
                "EXT-X-MAP" => return Err(unsupported("fMP4 segments (EXT-X-MAP) are not read")),
                "EXT-X-BYTERANGE" => return Err(unsupported("byte-range segments are not read")),
                "EXT-X-STREAM-INF" => {
                    saw_variant = true;
                    let playable = attribute(value, "CODECS").is_none_or(|c| {
                        c.split(',').map(str::trim).all(|c| {
                            c.starts_with("avc1") || c.starts_with("mp4a") || c.eq_ignore_ascii_case("mp3")
                        })
                    });
                    let bandwidth = attribute(value, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
                    variant = Some(playable.then_some(bandwidth));
                }
                _ => {}
            }
        } else if let Some(v) = variant.take() {
            if let Some(bw) = v
                && best.as_ref().is_none_or(|(b, _)| bw > *b)
            {
                best = Some((bw, resolve(url, line)));
            }
        } else {
            let d = duration.take().unwrap_or(Duration::ZERO);
            segments.push(Segment {
                url: resolve(url, line),
                sequence,
                duration: d,
                start,
                discontinuity: std::mem::take(&mut discontinuity),
            });
            sequence += 1;
            start += d;
        }
    }
    if saw_variant {
        return best
            .map(|(_, u)| Playlist::Master(u))
            .ok_or_else(|| unsupported("variants are all in codecs other than H.264, AAC and MP3"));
    }
    Ok(Playlist::Media(MediaPlaylist { target, segments, ended }))
}

/// The value of `name` in an attribute list (`BANDWIDTH=1280000,CODECS="avc1.4d401f,mp4a.40.2"`),
/// without quotes.
fn attribute(list: &str, name: &str) -> Option<String> {
    let mut rest = list;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, next) = if let Some(q) = after.strip_prefix('"') {
            let end = q.find('"')?;
            (&q[..end], q[end + 1..].trim_start_matches(','))
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (&after[..end], after[end..].trim_start_matches(','))
        };
        if key.trim() == name {
            return Some(value.to_string());
        }
        rest = next;
    }
    None
}

/// Whether `s` starts with a URL scheme (`https:`, `file:`).
fn has_scheme(s: &str) -> bool {
    s.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

/// `rel` (a URI in a playlist) as an absolute URL, read against the playlist's `base`.
pub(crate) fn resolve(base: &str, rel: &str) -> String {
    if has_scheme(rel) {
        return rel.to_string();
    }
    let base = base.split(['?', '#']).next().unwrap_or(base);
    let (origin, path) = match base.find("://") {
        Some(i) => {
            let host_end = base[i + 3..].find('/').map_or(base.len(), |j| i + 3 + j);
            (&base[..host_end], &base[host_end..])
        }
        None => ("", base),
    };
    if let Some(r) = rel.strip_prefix("//") {
        let scheme = base.split_once(':').map_or("", |(s, _)| s);
        return format!("{scheme}://{r}");
    }
    let split = rel.find(['?', '#']).unwrap_or(rel.len());
    let (rel_path, suffix) = rel.split_at(split);
    let joined = if rel_path.starts_with('/') {
        rel_path.to_string()
    } else {
        let dir = path.rfind('/').map_or(if origin.is_empty() { "" } else { "/" }, |i| &path[..=i]);
        format!("{dir}{rel_path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    let segs: Vec<&str> = joined.split('/').collect();
    for (i, s) in segs.iter().enumerate() {
        match *s {
            "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            _ => parts.push(s),
        }
        // A path ending in `.` or `..` names a directory.
        if i == segs.len() - 1 && (*s == "." || *s == "..") {
            parts.push("");
        }
    }
    format!("{origin}{}{suffix}", parts.join("/"))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests index what they have just checked is there")]
mod tests {
    use super::*;

    const BASE: &str = "https://example.com/live/stream/index.m3u8?token=abc";

    fn media(text: &str) -> MediaPlaylist {
        match parse(text, BASE) {
            Ok(Playlist::Media(m)) => m,
            other => unreachable!("a media playlist: {other:?}"),
        }
    }

    #[test]
    fn a_media_playlist_lists_its_segments_with_their_starts() {
        let m = media(
            "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:7\n\
             #EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXTINF:2.000000,\nseg00007.ts\n\
             #EXTINF:1.5,title\nseg00008.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:2,\nseg00009.ts\n",
        );
        assert_eq!(m.target, Duration::from_secs(2));
        assert!(!m.ended);
        let seqs: Vec<u64> = m.segments.iter().map(|s| s.sequence).collect();
        assert_eq!(seqs, [7, 8, 9]);
        assert_eq!(m.segments[1].start, Duration::from_secs(2));
        assert_eq!(m.segments[2].start, Duration::from_millis(3500));
        assert_eq!(m.segments[1].duration, Duration::from_millis(1500));
        assert_eq!(m.segments[0].url, "https://example.com/live/stream/seg00007.ts");
        assert!(!m.segments[1].discontinuity && m.segments[2].discontinuity);
        let m = media("#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST\n");
        assert!(m.ended);
    }

    #[test]
    fn uris_are_resolved_against_the_playlist() {
        assert_eq!(resolve(BASE, "a.ts"), "https://example.com/live/stream/a.ts");
        assert_eq!(resolve(BASE, "a.ts?x=1"), "https://example.com/live/stream/a.ts?x=1");
        assert_eq!(resolve(BASE, "../other/a.ts"), "https://example.com/live/other/a.ts");
        assert_eq!(resolve(BASE, "./sub/./a.ts"), "https://example.com/live/stream/sub/a.ts");
        assert_eq!(resolve(BASE, "../../../../a.ts"), "https://example.com/a.ts");
        assert_eq!(resolve(BASE, "/root/a.ts"), "https://example.com/root/a.ts");
        assert_eq!(resolve(BASE, "//cdn.example.net/a.ts"), "https://cdn.example.net/a.ts");
        assert_eq!(resolve(BASE, "http://other.example/x/a.ts"), "http://other.example/x/a.ts");
        assert_eq!(resolve("http://h:8080/index.m3u8", "seg1.ts"), "http://h:8080/seg1.ts");
        assert_eq!(resolve("http://h", "seg1.ts"), "http://h/seg1.ts");
        assert_eq!(resolve("file:///C:/media/hls/index.m3u8", "seg.ts"), "file:///C:/media/hls/seg.ts");
    }

    #[test]
    fn a_master_playlist_picks_the_best_playable_variant() {
        let text = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,CODECS=\"avc1.4d401f,mp4a.40.2\",RESOLUTION=640x360\nlow/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=9000000,CODECS=\"hvc1.1.6.L93.B0,mp4a.40.2\"\nhevc/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2400000,CODECS=\"avc1.640028,mp4a.40.2\"\nhigh/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1200000\nnocodecs.m3u8\n";
        assert_eq!(
            parse(text, BASE),
            Ok(Playlist::Master("https://example.com/live/stream/high/index.m3u8".into()))
        );
        let only_hevc = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"hvc1.1.6.L93.B0\"\nx.m3u8\n";
        assert!(matches!(parse(only_hevc, BASE), Err(MediaError::Unsupported(_))));
    }

    #[test]
    fn what_would_play_wrongly_is_refused() {
        let enc = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:1,\na.ts\n";
        assert!(matches!(parse(enc, BASE), Err(MediaError::Unsupported(r)) if r.contains("encrypted")));
        let clear = "#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:1,\na.ts\n";
        assert!(parse(clear, BASE).is_ok());
        let fmp4 = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\na.m4s\n";
        assert!(matches!(parse(fmp4, BASE), Err(MediaError::Unsupported(r)) if r.contains("fMP4")));
        let ranges = "#EXTM3U\n#EXTINF:1,\n#EXT-X-BYTERANGE:1000@0\na.ts\n";
        assert!(matches!(parse(ranges, BASE), Err(MediaError::Unsupported(_))));
        assert!(matches!(parse("<html>", BASE), Err(MediaError::Format(_))));
        assert!(matches!(parse("", BASE), Err(MediaError::Format(_))));
    }
}
