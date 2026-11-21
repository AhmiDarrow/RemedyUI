//! HLS in a page: the crate's own reader ([`crate::hls`]) takes the playlist and its MPEG-TS
//! segments apart, the page fetches what it asks for, and the browser's WebCodecs decoders
//! decode the H.264 access units (Annex B, so configured without a description) and the AAC
//! or MP3 frames. Decoded samples come out through the MP4 path's output handling.

use super::{
    AudioDecoder, EncodedAudioChunk, EncodedVideoChunk, Shared, Slot, VideoDecoder, audio_output, has_global,
    js_error, obj, video_output,
};
use crate::hls::{AudioCodec, AudioConfig, Elementary, Unit, VideoConfig};
use crate::host::{MediaError, Sample};
use js_sys::{Object, Promise, Uint8Array};
use std::time::Duration;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// The browser's decoders for the compressed streams of an HLS source.
pub(super) struct WebElementary {
    out: Shared,
    video: Option<(VideoDecoder, Object)>,
    audio: Option<(AudioDecoder, Object)>,
    _callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
}

// SAFETY: as for the MP4 decoder: one thread in a page without shared memory.
unsafe impl Send for WebElementary {}

fn us(d: Duration) -> JsValue {
    JsValue::from_f64(d.as_secs_f64() * 1_000_000.0)
}

impl WebElementary {
    pub(super) fn new(out: Shared) -> WebElementary {
        WebElementary { out, video: None, audio: None, _callbacks: Vec::new() }
    }

    /// Whether the decoder `unit` goes to is still busy with the last two (as the MP4 feed).
    pub(super) fn busy(&self, unit: &Unit) -> bool {
        match unit {
            Unit::Video { .. } => self.video.as_ref().is_some_and(|(d, _)| d.decode_queue_size() >= 2),
            Unit::Audio { .. } => self.audio.as_ref().is_some_and(|(d, _)| d.decode_queue_size() >= 2),
        }
    }

    /// Everything pushed has come out and been read (after [`Elementary::end`]).
    pub(super) fn drained(&self) -> bool {
        let o = self.out.borrow();
        o.flushed && o.queue.is_empty()
    }
}

impl Elementary for WebElementary {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        let error_cb = {
            let out = self.out.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
                out.borrow_mut().fail(MediaError::Decode(js_error("the browser's decoder failed", &e)));
            })
        };
        let dequeue = {
            let out = self.out.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |_| out.borrow_mut().wake())
        };
        if let Some(v) = video {
            if !has_global("VideoDecoder") {
                return Err(MediaError::Unsupported("this browser has no WebCodecs video decoder".into()));
            }
            let out = self.out.clone();
            let output =
                Closure::<dyn FnMut(JsValue)>::new(move |f: JsValue| video_output(&out, f.unchecked_into()));
            let init = obj(&[("output", output.as_ref().clone()), ("error", error_cb.as_ref().clone())]);
            let dec = VideoDecoder::new(&init)
                .map_err(|e| MediaError::Unsupported(js_error("video decoding", &e)))?;
            let _ = js_sys::Reflect::set(&dec, &"ondequeue".into(), dequeue.as_ref());
            // No description: the access units come as an Annex B stream with their SPS and PPS.
            let config = obj(&[
                ("codec", JsValue::from_str(&v.codec)),
                ("codedWidth", JsValue::from(v.width)),
                ("codedHeight", JsValue::from(v.height)),
                ("optimizeForLatency", JsValue::TRUE),
            ]);
            dec.configure(&config)
                .map_err(|e| MediaError::Unsupported(js_error(&format!("decoding {}", v.codec), &e)))?;
            self._callbacks.push(output);
            self.video = Some((dec, config));
        }
        if let Some(a) = audio {
            if !has_global("AudioDecoder") {
                return Err(MediaError::Unsupported("this browser has no WebCodecs sound decoder".into()));
            }
            let out = self.out.clone();
            let output = Closure::<dyn FnMut(JsValue)>::new(move |d: JsValue| {
                let data: super::AudioData = d.unchecked_into();
                if let Err(e) = audio_output(&out, &data) {
                    out.borrow_mut().fail(e);
                }
                data.close_data();
            });
            let init = obj(&[("output", output.as_ref().clone()), ("error", error_cb.as_ref().clone())]);
            let dec = AudioDecoder::new(&init)
                .map_err(|e| MediaError::Unsupported(js_error("sound decoding", &e)))?;
            let _ = js_sys::Reflect::set(&dec, &"ondequeue".into(), dequeue.as_ref());
            let mut pairs = vec![
                ("codec", JsValue::from_str(&a.codec_string)),
                ("sampleRate", JsValue::from(a.sample_rate)),
                ("numberOfChannels", JsValue::from(a.channels)),
            ];
            if let AudioCodec::Aac { asc } = &a.codec {
                pairs.push(("description", Uint8Array::from(asc.as_slice()).into()));
            }
            let config = obj(&pairs);
            dec.configure(&config).map_err(|e| {
                MediaError::Unsupported(js_error(&format!("decoding {}", a.codec_string), &e))
            })?;
            self._callbacks.push(output);
            self.audio = Some((dec, config));
        }
        self._callbacks.push(error_cb);
        self._callbacks.push(dequeue);
        Ok(())
    }

    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        match unit {
            Unit::Video { pts, keyframe, data, .. } => {
                let Some((dec, _)) = &self.video else { return Ok(()) };
                let init = obj(&[
                    ("type", JsValue::from_str(if keyframe { "key" } else { "delta" })),
                    ("timestamp", us(pts)),
                    ("data", Uint8Array::from(data.as_slice()).into()),
                ]);
                let chunk = EncodedVideoChunk::new(&init)
                    .map_err(|e| MediaError::Decode(js_error("a picture", &e)))?;
                dec.decode(&chunk).map_err(|e| MediaError::Decode(js_error("decoding a picture", &e)))
            }
            Unit::Audio { pts, data } => {
                let Some((dec, _)) = &self.audio else { return Ok(()) };
                let init = obj(&[
                    ("type", JsValue::from_str("key")),
                    ("timestamp", us(pts)),
                    ("data", Uint8Array::from(data.as_slice()).into()),
                ]);
                let chunk =
                    EncodedAudioChunk::new(&init).map_err(|e| MediaError::Decode(js_error("sound", &e)))?;
                dec.decode(&chunk).map_err(|e| MediaError::Decode(js_error("decoding sound", &e)))
            }
        }
    }

    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        let mut o = self.out.borrow_mut();
        if let Some(e) = o.error.take() {
            return Err(e);
        }
        if matches!(o.queue.front(), Some(Slot::Ready(_)))
            && let Some(Slot::Ready(s)) = o.queue.pop_front()
        {
            return Ok(Some(s));
        }
        Ok(None)
    }

    fn end(&mut self) -> Result<(), MediaError> {
        let flushes: Vec<Promise> = self
            .video
            .as_ref()
            .and_then(|(d, _)| d.flush().ok())
            .into_iter()
            .chain(self.audio.as_ref().and_then(|(d, _)| d.flush().ok()))
            .collect();
        let (out, generation) = (self.out.clone(), self.out.borrow().generation);
        wasm_bindgen_futures::spawn_local(async move {
            for f in flushes {
                // A reset (a seek) rejects a flush under way; the generation tells.
                let _ = JsFuture::from(f).await;
            }
            let mut o = out.borrow_mut();
            if o.generation == generation {
                o.flushed = true;
                o.wake();
            }
        });
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        {
            let mut o = self.out.borrow_mut();
            o.generation += 1;
            o.queue.clear();
            o.flushed = false;
            o.error = None;
        }
        let seeking = |e: JsValue| MediaError::Decode(js_error("seeking", &e));
        if let Some((d, config)) = &self.video {
            d.reset().map_err(seeking)?;
            d.configure(config).map_err(seeking)?;
        }
        if let Some((d, config)) = &self.audio {
            d.reset().map_err(seeking)?;
            d.configure(config).map_err(seeking)?;
        }
        Ok(())
    }
}

impl Drop for WebElementary {
    fn drop(&mut self) {
        if let Some((d, _)) = &self.video {
            let _ = d.close();
        }
        if let Some((d, _)) = &self.audio {
            let _ = d.close();
        }
    }
}
