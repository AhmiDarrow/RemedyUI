//! The helper: shuts itself in before it reads anything, says so, then decodes what it is
//! sent until the app goes. It decodes with `remedy-ui-media::media_image` (and, for animations,
//! `remedy-ui-media::media_anim`) in this process (no router or opener is ever set here), and the
//! formats the app registered before `helper_main`.
//!
//! It keeps each open animation's decoder (with its own copy of the file's bytes) under a
//! session number until the app closes it.

use std::collections::HashMap;

use crate::media_image::{DecodeError, decode, dimensions, is_scalable};

use crate::media_isolated::os::{Mapped, Parent, layer_bits, lock_down};
use crate::media_isolated::protocol::{Reply, Request};

#[cfg(feature = "media-isolated-anim")]
type Session = Box<dyn crate::media_anim::route::FrameSource>;
#[cfg(not(feature = "media-isolated-anim"))]
type Session = ();

/// Runs the helper; the process's exit code.
pub(crate) fn run() -> i32 {
    let Ok(mut parent) = Parent::open() else { return 3 };
    let layers = match lock_down() {
        Ok(l) => l,
        Err(why) => {
            let _ = parent.send(&Reply::Refused(why).encode());
            return 4;
        }
    };
    if parent.send(&Reply::Ready { layers: layer_bits(&layers) }.encode()).is_err() {
        return 0;
    }
    let mut sessions: HashMap<u32, Session> = HashMap::new();
    let mut next_session: u32 = 0;
    loop {
        // The app closing the channel is the normal way out.
        let Ok((payload, mut regions)) = parent.recv() else { return 0 };
        let Some(request) = Request::decode(&payload) else { return 5 };
        let reply = answer(request, &mut regions, &mut sessions, &mut next_session);
        drop(regions);
        if parent.send(&reply.encode()).is_err() {
            return 0;
        }
    }
}

fn input(regions: &[Mapped], len: u64) -> Result<&[u8], DecodeError> {
    let bad = || DecodeError::Io("the request's bytes are missing".into());
    let len = usize::try_from(len).map_err(|_| bad())?;
    regions.first().and_then(|r| r.bytes().get(..len)).ok_or_else(bad)
}

fn answer(
    request: Request,
    regions: &mut [Mapped],
    sessions: &mut HashMap<u32, Session>,
    next_session: &mut u32,
) -> Reply {
    let result = match request {
        Request::Probe { len, animated } => input(regions, len).and_then(|b| {
            let size = if animated { probe_animation(b)? } else { dimensions(b)? };
            Ok(Reply::Size { size, scalable: is_scalable(b) })
        }),
        Request::Decode { len, show } => decode_still(regions, len, show),
        Request::Open { len, box_px, cover } => {
            input(regions, len).and_then(|b| open(b, box_px, cover, sessions, next_session))
        }
        Request::Next { session } => next(sessions, session, regions.get_mut(0)),
        Request::Skip { session, count } => skip(sessions, session, count),
        Request::Restart { session } => restart(sessions, session),
        Request::Close { session } => {
            sessions.remove(&session);
            Ok(Reply::Done)
        }
    };
    result.unwrap_or_else(Reply::Failed)
}

fn decode_still(regions: &mut [Mapped], len: u64, show: (u32, u32)) -> Result<Reply, DecodeError> {
    let decoded = decode(input(regions, len)?, Some(show))?;
    let img = &decoded.image;
    if img.width > show.0 || img.height > show.1 {
        return Err(DecodeError::Malformed(format!(
            "decoded to {}×{}, larger than the {}×{} asked for",
            img.width, img.height, show.0, show.1
        )));
    }
    let out =
        regions.get_mut(1).ok_or_else(|| DecodeError::Io("the request has nowhere for pixels".into()))?;
    if !out.write(&img.pixels) {
        return Err(DecodeError::TooLarge { width: img.width, height: img.height });
    }
    Ok(Reply::Pixels { size: (img.width, img.height), source: decoded.source_size })
}

fn no_session(session: u32) -> DecodeError {
    DecodeError::Io(format!("no animation {session} is open"))
}

#[cfg(feature = "media-isolated-anim")]
fn probe_animation(bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
    crate::media_anim::route::dimensions_here(bytes)
}

#[cfg(feature = "media-isolated-anim")]
fn open(
    bytes: &[u8],
    box_px: (u32, u32),
    cover: bool,
    sessions: &mut HashMap<u32, Session>,
    next_session: &mut u32,
) -> Result<Reply, DecodeError> {
    use crate::media_anim::route::{Loops, Opened, open_here};
    // The decoder keeps the bytes: copy them out of the region, which goes after this.
    let bytes: std::sync::Arc<[u8]> = std::sync::Arc::from(bytes);
    match open_here(&bytes, box_px, cover)? {
        Opened::Still => Ok(Reply::Still),
        Opened::Animated(source) => {
            *next_session = next_session.wrapping_add(1);
            let session = *next_session;
            let a = *source.animation();
            sessions.insert(session, source);
            Ok(Reply::Animation(crate::media_isolated::protocol::Opened {
                session,
                size: a.size,
                show_at: a.show_at,
                frames: a.frames,
                loops: match a.loops {
                    Loops::Forever => None,
                    Loops::Times(n) => Some(n),
                },
            }))
        }
    }
}

#[cfg(feature = "media-isolated-anim")]
fn next(
    sessions: &mut HashMap<u32, Session>,
    session: u32,
    out: Option<&mut Mapped>,
) -> Result<Reply, DecodeError> {
    let source = sessions.get_mut(&session).ok_or_else(|| no_session(session))?;
    let show = source.animation().show_at;
    let Some((image, delay)) = source.next()? else { return Ok(Reply::End) };
    if image.width > show.0 || image.height > show.1 {
        return Err(DecodeError::Malformed(format!(
            "a frame came at {}×{}, larger than the {}×{} asked for",
            image.width, image.height, show.0, show.1
        )));
    }
    let out = out.ok_or_else(|| DecodeError::Io("the request has nowhere for pixels".into()))?;
    if !out.write(&image.pixels) {
        return Err(DecodeError::TooLarge { width: image.width, height: image.height });
    }
    let delay_us = u64::try_from(delay.as_micros()).unwrap_or(u64::MAX);
    Ok(Reply::Frame { size: (image.width, image.height), delay_us })
}

#[cfg(feature = "media-isolated-anim")]
fn skip(sessions: &mut HashMap<u32, Session>, session: u32, count: u64) -> Result<Reply, DecodeError> {
    let source = sessions.get_mut(&session).ok_or_else(|| no_session(session))?;
    for _ in 0..count {
        if source.next()?.is_none() {
            return Ok(Reply::End);
        }
    }
    Ok(Reply::Done)
}

#[cfg(feature = "media-isolated-anim")]
fn restart(sessions: &mut HashMap<u32, Session>, session: u32) -> Result<Reply, DecodeError> {
    sessions.get_mut(&session).ok_or_else(|| no_session(session))?.restart()?;
    Ok(Reply::Done)
}

#[cfg(not(feature = "media-isolated-anim"))]
fn without_anim() -> DecodeError {
    DecodeError::Io(
        "this app's helper decodes no animations (the `media-isolated-anim` feature is off)".into(),
    )
}

#[cfg(not(feature = "media-isolated-anim"))]
fn probe_animation(_bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
    Err(without_anim())
}

#[cfg(not(feature = "media-isolated-anim"))]
fn open(
    _bytes: &[u8],
    _box_px: (u32, u32),
    _cover: bool,
    _sessions: &mut HashMap<u32, Session>,
    _next_session: &mut u32,
) -> Result<Reply, DecodeError> {
    Err(without_anim())
}

#[cfg(not(feature = "media-isolated-anim"))]
fn next(
    sessions: &mut HashMap<u32, Session>,
    session: u32,
    _out: Option<&mut Mapped>,
) -> Result<Reply, DecodeError> {
    let _ = sessions;
    Err(no_session(session))
}

#[cfg(not(feature = "media-isolated-anim"))]
fn skip(_sessions: &mut HashMap<u32, Session>, session: u32, _count: u64) -> Result<Reply, DecodeError> {
    Err(no_session(session))
}

#[cfg(not(feature = "media-isolated-anim"))]
fn restart(_sessions: &mut HashMap<u32, Session>, session: u32) -> Result<Reply, DecodeError> {
    Err(no_session(session))
}
