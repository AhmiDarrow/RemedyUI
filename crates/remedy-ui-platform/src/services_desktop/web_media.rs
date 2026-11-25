//! The page's `navigator.mediaSession`, reached by name (web-sys keeps it behind its unstable
//! APIs).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::services_desktop::DesktopError;
use crate::services_desktop::media_session::{Artwork, Handler, MediaCommand, MediaState, NowPlaying};

/// Seconds a seek button moves when the browser does not say.
const SKIP: f64 = 10.0;

type Action = Closure<dyn Fn(JsValue)>;

pub(crate) struct Session {
    ms: JsValue,
    actions: Vec<(&'static str, Action)>,
    next: Action,
    art_url: RefCell<Option<String>>,
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn set(o: &JsValue, k: &str, v: &JsValue) {
    let _ = Reflect::set(o, &JsValue::from_str(k), v);
}

fn number(details: &JsValue, k: &str) -> Option<f64> {
    get(details, k).as_f64().filter(|n| n.is_finite())
}

impl Session {
    pub(crate) fn new(handler: Handler) -> Result<Session, DesktopError> {
        let ms = web_sys::window().map(|w| get(&w.navigator(), "mediaSession")).unwrap_or(JsValue::UNDEFINED);
        if ms.is_undefined() || ms.is_null() {
            return Err(DesktopError::unsupported(
                "Media controls",
                "this browser has no navigator.mediaSession",
            ));
        }
        let handler: Rc<Handler> = Rc::new(handler);
        let action = |f: fn(&JsValue) -> Option<MediaCommand>| {
            let h = handler.clone();
            Closure::<dyn Fn(JsValue)>::new(move |d: JsValue| {
                if let Some(c) = f(&d) {
                    h(c);
                }
            })
        };
        let actions: Vec<(&'static str, Action)> = vec![
            ("play", action(|_| Some(MediaCommand::Play))),
            ("pause", action(|_| Some(MediaCommand::Pause))),
            ("stop", action(|_| Some(MediaCommand::Stop))),
            ("previoustrack", action(|_| Some(MediaCommand::Previous))),
            (
                "seekbackward",
                action(|d| Some(MediaCommand::SeekBy(-number(d, "seekOffset").unwrap_or(SKIP)))),
            ),
            ("seekforward", action(|d| Some(MediaCommand::SeekBy(number(d, "seekOffset").unwrap_or(SKIP))))),
            (
                "seekto",
                action(|d| {
                    number(d, "seekTime")
                        .and_then(|s| Duration::try_from_secs_f64(s).ok())
                        .map(MediaCommand::SeekTo)
                }),
            ),
        ];
        let next = action(|_| Some(MediaCommand::Next));
        let s = Session { ms, actions, next, art_url: RefCell::new(None) };
        for (name, f) in &s.actions {
            s.set_action(name, Some(f));
        }
        Ok(s)
    }

    /// Sets (or clears) one action's handler; a browser that does not know the action throws,
    /// which leaves it out.
    fn set_action(&self, name: &str, f: Option<&Action>) {
        if let Ok(set) = get(&self.ms, "setActionHandler").dyn_into::<Function>() {
            let f = f.map_or(JsValue::NULL, |f| f.as_ref().clone());
            let _ = set.call2(&self.ms, &JsValue::from_str(name), &f);
        }
    }

    fn drop_art(&self) {
        if let Some(u) = self.art_url.borrow_mut().take() {
            let _ = web_sys::Url::revoke_object_url(&u);
        }
    }

    pub(crate) fn now_playing(&self, now: Option<&NowPlaying>) {
        self.drop_art();
        let meta = now.and_then(|n| {
            let init = Object::new();
            set(&init, "title", &JsValue::from_str(&n.title));
            if let Some(s) = &n.subtitle {
                set(&init, "artist", &JsValue::from_str(s));
            }
            let src = match &n.art {
                Some(Artwork::Url(u)) => Some(u.clone()),
                Some(Artwork::Bytes(b)) => {
                    let parts = Array::of1(&js_sys::Uint8Array::from(b.as_slice()));
                    let url = web_sys::Blob::new_with_u8_array_sequence(&parts)
                        .and_then(|blob| web_sys::Url::create_object_url_with_blob(&blob))
                        .ok();
                    self.art_url.replace(url.clone());
                    url
                }
                None => None,
            };
            if let Some(src) = src {
                let image = Object::new();
                set(&image, "src", &JsValue::from_str(&src));
                set(&init, "artwork", &Array::of1(&image));
            }
            let ctor = web_sys::window().map(|w| get(&w, "MediaMetadata"))?.dyn_into::<Function>().ok()?;
            Reflect::construct(&ctor, &Array::of1(&init)).ok()
        });
        set(&self.ms, "metadata", &meta.unwrap_or(JsValue::NULL));
        if now.is_none() {
            // Out of the controls, as Windows' SMTC is turned off.
            set(&self.ms, "playbackState", &JsValue::from_str("none"));
        }
    }

    pub(crate) fn playback(&self, state: MediaState, position: Duration, length: Option<Duration>) {
        let s = match state {
            MediaState::Playing => "playing",
            MediaState::Paused => "paused",
            MediaState::Stopped => "none",
        };
        set(&self.ms, "playbackState", &JsValue::from_str(s));
        let Ok(f) = get(&self.ms, "setPositionState").dyn_into::<Function>() else { return };
        let _ = match length.filter(|l| !l.is_zero()) {
            Some(l) => {
                let o = Object::new();
                set(&o, "duration", &JsValue::from_f64(l.as_secs_f64()));
                set(&o, "position", &JsValue::from_f64(position.min(l).as_secs_f64()));
                set(&o, "playbackRate", &JsValue::from_f64(1.0));
                f.call1(&self.ms, &o)
            }
            // A live stream: no seek bar.
            None => f.call0(&self.ms),
        };
    }

    pub(crate) fn can_next(&self, can: bool) {
        self.set_action("nexttrack", can.then_some(&self.next));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for (name, _) in &self.actions {
            self.set_action(name, None);
        }
        self.set_action("nexttrack", None);
        self.now_playing(None);
    }
}
