//! What the app and a helper say to each other. Every message is one frame of at most
//! `u16::MAX` bytes; pixels and file bytes travel in shared memory regions passed with it.
//! The app reads the helper's replies as hostile: every field is checked before use.
//!
//! Stills take one request each. An animation is a session: opened once (the helper keeps
//! its decoder and the file's bytes), then asked for one frame at a time, skipped forward,
//! started again for the next loop, and closed.

use remedy_ui_media_image::DecodeError;

/// Longest text a reply carries (a decoder's error message), in bytes: what fits in a frame
/// with room to spare.
pub(crate) const MAX_TEXT: usize = 4096;

/// From the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    /// Read the size of the `len` bytes at the start of region 0 (as an animation when
    /// `animated`: animated formats the app added are read too).
    Probe { len: u64, animated: bool },
    /// Decode the `len` bytes at the start of region 0 at `show` pixels into region 1.
    Decode { len: u64, show: (u32, u32) },
    /// Open the `len` bytes at the start of region 0 as an animation to play in a `box_px`
    /// box, fitting it or (`cover`) covering it.
    Open { len: u64, box_px: (u32, u32), cover: bool },
    /// The session's next frame, into region 0.
    Next { session: u32 },
    /// Read past the session's next `count` frames.
    Skip { session: u32, count: u64 },
    /// Back to the session's first frame.
    Restart { session: u32 },
    /// The session is over.
    Close { session: u32 },
}

/// An animation as opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Opened {
    pub session: u32,
    pub size: (u32, u32),
    pub show_at: (u32, u32),
    pub frames: Option<u64>,
    /// How many plays; `None` is without end.
    pub loops: Option<u32>,
}

/// From the helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Shut in and waiting, under these layers (a bit set, see [`crate::os::layer_bits`]).
    Ready { layers: u32 },
    /// It could not shut itself in, and why; it has quit.
    Refused(String),
    /// The image's natural size, and whether it draws sharply at any size.
    Size { size: (u32, u32), scalable: bool },
    /// The pixels are in region 1: `size` of them, from a `source`-sized picture.
    Pixels { size: (u32, u32), source: (u32, u32) },
    /// It could not be read.
    Failed(DecodeError),
    /// An animation is open.
    Animation(Opened),
    /// The file is a still: the still loader shows it.
    Still,
    /// A frame is in region 0: `size` pixels, shown for `delay_us` microseconds.
    Frame { size: (u32, u32), delay_us: u64 },
    /// The play has no more frames.
    End,
    /// Done (skipped, restarted, closed).
    Done,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }
    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u16(&mut self) -> Option<u16> {
        self.bytes().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes().map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.bytes().map(u64::from_le_bytes)
    }
    fn pair(&mut self) -> Option<(u32, u32)> {
        Some((self.u32()?, self.u32()?))
    }
    fn flag(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
    fn text(&mut self) -> Option<String> {
        let n = usize::from(self.u16()?);
        if n > MAX_TEXT {
            return None;
        }
        let (s, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(String::from_utf8_lossy(s).into_owned())
    }
    fn end(&self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn new(op: u8) -> Writer {
        Writer(vec![op])
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn pair(self, p: (u32, u32)) -> Self {
        self.u32(p.0).u32(p.1)
    }
    fn flag(self, b: bool) -> Self {
        self.u8(u8::from(b))
    }
    fn text(mut self, s: &str) -> Self {
        let mut n = s.len().min(MAX_TEXT);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.0.extend_from_slice(&(n as u16).to_le_bytes());
        self.0.extend_from_slice(s.as_bytes().get(..n).unwrap_or_default());
        self
    }
}

/// `None` (no count, or without end) travels as `u64::MAX`.
const NONE64: u64 = u64::MAX;

impl Request {
    pub(crate) fn encode(&self) -> Vec<u8> {
        match *self {
            Request::Probe { len, animated } => Writer::new(1).u64(len).flag(animated),
            Request::Decode { len, show } => Writer::new(2).u64(len).pair(show),
            Request::Open { len, box_px, cover } => Writer::new(3).u64(len).pair(box_px).flag(cover),
            Request::Next { session } => Writer::new(4).u32(session),
            Request::Skip { session, count } => Writer::new(5).u32(session).u64(count),
            Request::Restart { session } => Writer::new(6).u32(session),
            Request::Close { session } => Writer::new(7).u32(session),
        }
        .0
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Request> {
        let mut r = Reader(bytes);
        let req = match r.u8()? {
            1 => Request::Probe { len: r.u64()?, animated: r.flag()? },
            2 => Request::Decode { len: r.u64()?, show: r.pair()? },
            3 => Request::Open { len: r.u64()?, box_px: r.pair()?, cover: r.flag()? },
            4 => Request::Next { session: r.u32()? },
            5 => Request::Skip { session: r.u32()?, count: r.u64()? },
            6 => Request::Restart { session: r.u32()? },
            7 => Request::Close { session: r.u32()? },
            _ => return None,
        };
        r.end()?;
        Some(req)
    }
}

impl Reply {
    pub(crate) fn encode(&self) -> Vec<u8> {
        match self {
            Reply::Ready { layers } => Writer::new(0).u32(*layers),
            Reply::Refused(why) => Writer::new(1).text(why),
            Reply::Size { size, scalable } => Writer::new(2).pair(*size).flag(*scalable),
            Reply::Pixels { size, source } => Writer::new(3).pair(*size).pair(*source),
            Reply::Failed(e) => {
                let (kind, w, h, text) = match e {
                    DecodeError::Unrecognized => (0, 0, 0, ""),
                    DecodeError::Malformed(s) => (1, 0, 0, s.as_str()),
                    DecodeError::TooLarge { width, height } => (2, *width, *height, ""),
                    DecodeError::Io(s) => (3, 0, 0, s.as_str()),
                    DecodeError::Stopped(s) => (4, 0, 0, s.as_str()),
                };
                Writer::new(4).u8(kind).u32(w).u32(h).text(text)
            }
            Reply::Animation(o) => Writer::new(5)
                .u32(o.session)
                .pair(o.size)
                .pair(o.show_at)
                .u64(o.frames.unwrap_or(NONE64))
                .u64(o.loops.map_or(NONE64, u64::from)),
            Reply::Still => Writer::new(6),
            Reply::Frame { size, delay_us } => Writer::new(7).pair(*size).u64(*delay_us),
            Reply::End => Writer::new(8),
            Reply::Done => Writer::new(9),
        }
        .0
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Reply> {
        let mut r = Reader(bytes);
        let reply = match r.u8()? {
            0 => Reply::Ready { layers: r.u32()? },
            1 => Reply::Refused(r.text()?),
            2 => Reply::Size { size: r.pair()?, scalable: r.flag()? },
            3 => Reply::Pixels { size: r.pair()?, source: r.pair()? },
            4 => {
                let kind = r.u8()?;
                let (w, h) = r.pair()?;
                let text = r.text()?;
                Reply::Failed(match kind {
                    0 => DecodeError::Unrecognized,
                    1 => DecodeError::Malformed(text),
                    2 => DecodeError::TooLarge { width: w, height: h },
                    3 => DecodeError::Io(text),
                    // A helper never reports itself stopped; that is the app's to say.
                    _ => return None,
                })
            }
            5 => {
                let session = r.u32()?;
                let (size, show_at) = (r.pair()?, r.pair()?);
                let frames = Some(r.u64()?).filter(|&f| f != NONE64);
                let loops = match r.u64()? {
                    NONE64 => None,
                    n => Some(u32::try_from(n).ok()?),
                };
                Reply::Animation(Opened { session, size, show_at, frames, loops })
            }
            6 => Reply::Still,
            7 => Reply::Frame { size: r.pair()?, delay_us: r.u64()? },
            8 => Reply::End,
            9 => Reply::Done,
            _ => return None,
        };
        r.end()?;
        Some(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_and_garbage_is_refused() {
        let requests = [
            Request::Probe { len: 7, animated: true },
            Request::Decode { len: 1 << 40, show: (640, 480) },
            Request::Open { len: 9, box_px: (40, 30), cover: true },
            Request::Next { session: 3 },
            Request::Skip { session: 3, count: 12 },
            Request::Restart { session: 4 },
            Request::Close { session: 5 },
        ];
        for req in requests {
            assert_eq!(Request::decode(&req.encode()), Some(req));
        }
        let replies = [
            Reply::Ready { layers: 0b101 },
            Reply::Refused("no seccomp".into()),
            Reply::Size { size: (3, 4), scalable: true },
            Reply::Pixels { size: (1, 2), source: (3, 4) },
            Reply::Failed(DecodeError::Malformed("bad".into())),
            Reply::Failed(DecodeError::TooLarge { width: 9, height: 8 }),
            Reply::Failed(DecodeError::Unrecognized),
            Reply::Animation(Opened {
                session: 1,
                size: (8, 8),
                show_at: (4, 4),
                frames: Some(3),
                loops: None,
            }),
            Reply::Animation(Opened {
                session: 2,
                size: (8, 8),
                show_at: (8, 8),
                frames: None,
                loops: Some(2),
            }),
            Reply::Still,
            Reply::Frame { size: (4, 4), delay_us: 100_000 },
            Reply::End,
            Reply::Done,
        ];
        for rep in replies {
            assert_eq!(Reply::decode(&rep.encode()), Some(rep));
        }
        // Trailing bytes, cut messages, unknown kinds and self-declared stops are refused.
        let mut long = Reply::Ready { layers: 1 }.encode();
        long.push(0);
        assert_eq!(Reply::decode(&long), None);
        assert_eq!(Reply::decode(&[2, 1, 0]), None);
        assert_eq!(Reply::decode(&[99]), None);
        assert_eq!(Reply::decode(&Reply::Failed(DecodeError::Stopped("x".into())).encode()), None);
        assert_eq!(Request::decode(&[2, 0]), None);
        assert_eq!(Request::decode(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 2]), None, "a flag is 0 or 1");
    }

    #[test]
    fn long_text_is_cut_at_a_character() {
        let s = "é".repeat(MAX_TEXT);
        let enc = Reply::Refused(s).encode();
        match Reply::decode(&enc) {
            Some(Reply::Refused(t)) => assert!(t.len() <= MAX_TEXT && t.chars().all(|c| c == 'é')),
            other => unreachable!("{other:?}"),
        }
    }
}
