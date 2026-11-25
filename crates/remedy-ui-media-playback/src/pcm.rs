//! Media as plain samples: a whole source decoded to mono at the rate an analysis wants
//! (pitch detection, a transcription model, a waveform), and the resampling that takes.

use crate::host::{MediaError, MediaHost, MediaSource};

/// Zero crossings of the resampling kernel on each side: 16 keeps the pass band flat to
/// within a hundredth of a decibel and what would alias more than 70 dB down.
const LOBES: f64 = 16.0;

/// The average of the channels of interleaved `samples`: one value per frame.
pub fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = usize::from(channels.max(1));
    if ch == 1 {
        return samples.to_vec();
    }
    samples.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
}

/// A Blackman window over -1..1.
fn window(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        return 0.0;
    }
    let t = std::f64::consts::PI * x;
    0.42 + 0.5 * t.cos() + 0.08 * (2.0 * t).cos()
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        return 1.0;
    }
    let t = std::f64::consts::PI * x;
    t.sin() / t
}

/// Interleaved `samples` (`channels` to a frame) at `from` frames a second, at `to`: a
/// band-limited (windowed sinc) conversion. Going down, everything above the new rate's
/// half is filtered out first, so it does not fold back into the sound as false tones (what
/// a plain interpolation does to 44.1 kHz music brought to 16 kHz).
pub fn resample(samples: &[f32], channels: u16, from: u32, to: u32) -> Vec<f32> {
    let ch = usize::from(channels.max(1));
    let frames = samples.len() / ch;
    if from == to || from == 0 || to == 0 || frames == 0 {
        return samples.get(..frames * ch).unwrap_or_default().to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    // The filter's edge, as a share of the input's own half rate: just under the lower of
    // the two rates' halves.
    let cutoff = ratio.min(1.0) * 0.97;
    let reach = LOBES / cutoff;
    let out_frames = ((frames as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_frames * ch);
    let mut weights: Vec<f64> = Vec::new();
    for i in 0..out_frames {
        let at = i as f64 / ratio;
        let first = (at - reach).ceil() as i64;
        let last = (at + reach).floor() as i64;
        weights.clear();
        weights.extend((first..=last).map(|k| {
            let d = at - k as f64;
            cutoff * sinc(cutoff * d) * window(d / reach)
        }));
        // The taps are normalised, so a steady level stays exactly that level.
        let sum: f64 = weights.iter().sum();
        let gain = if sum.abs() > 1e-12 { 1.0 / sum } else { 1.0 };
        for c in 0..ch {
            let mut v = 0.0f64;
            for (k, w) in (first..=last).zip(&weights) {
                // Past either end the sound holds its last frame.
                let frame = k.clamp(0, frames as i64 - 1) as usize;
                v += f64::from(samples.get(frame * ch + c).copied().unwrap_or(0.0)) * w;
            }
            out.push((v * gain) as f32);
        }
    }
    out
}

/// Decodes all of `source`'s sound to mono at `rate` frames a second, wherever the app runs
/// (in a browser page the browser decodes it, so anything it plays works: mp3, wav, ogg,
/// flac, m4a). It waits on the decoder, not on the thread: run it on the app's executor.
///
/// A long file is long in memory too: a minute at 16 kHz is about 4 MB.
pub async fn decode_pcm(
    host: &dyn MediaHost,
    source: &MediaSource,
    rate: u32,
) -> Result<Vec<f32>, MediaError> {
    if rate == 0 {
        return Err(MediaError::Format("a sample rate of zero".into()));
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let _ = host;
        crate::os::decode_audio(source, rate).await
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::future::poll_fn;

        use crate::host::{AudioFormat, Sample, open_source};
        let mut decoder = open_source(host, source)?;
        poll_fn(|cx| decoder.poll_ready(cx)).await?;
        let format: AudioFormat =
            decoder.info().audio.ok_or_else(|| MediaError::Format("the source has no sound".into()))?;
        let mut mono = Vec::new();
        while let Some(sample) = poll_fn(|cx| decoder.poll_read(cx)).await? {
            if let Sample::Audio { samples, .. } = sample {
                mono.extend(to_mono(&samples, format.channels));
            }
        }
        Ok(resample(&mono, 1, format.sample_rate, rate))
    }
}

/// [`decode_pcm`], waiting on this thread until it is done: for a worker thread or a tool.
/// (Not in a browser page, where nothing may wait on the page's thread.)
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn decode_pcm_blocking(
    host: &dyn MediaHost,
    source: &MediaSource,
    rate: u32,
) -> Result<Vec<f32>, MediaError> {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    /// Wakes the waiting thread.
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut work = std::pin::pin!(decode_pcm(host, source, rate));
    loop {
        match work.as_mut().poll(&mut cx) {
            Poll::Ready(done) => return done,
            // A decoder that is still opening wakes this thread; one that forgets to is
            // asked again shortly.
            Poll::Pending => std::thread::park_timeout(std::time::Duration::from_millis(5)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n).map(|i| (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin() as f32).collect()
    }

    /// The loudness (root mean square) of the middle of `s`, away from the ends.
    fn level(s: &[f32]) -> f32 {
        let mid = s.get(s.len() / 4..s.len() * 3 / 4).unwrap_or_default();
        (mid.iter().map(|v| v * v).sum::<f32>() / mid.len().max(1) as f32).sqrt()
    }

    #[test]
    fn a_tone_below_the_new_half_rate_survives_and_one_above_it_is_removed() {
        // 44.1 kHz to 16 kHz: 1 kHz is kept at full level; 10 kHz (above 8 kHz) would fold
        // to 6 kHz with plain interpolation, and here is gone.
        let kept = resample(&tone(1_000.0, 44_100, 0.5), 1, 44_100, 16_000);
        assert!((kept.len() as i64 - 8_000).abs() <= 1, "{}", kept.len());
        assert!((level(&kept) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01, "{}", level(&kept));
        let gone = resample(&tone(10_000.0, 44_100, 0.5), 1, 44_100, 16_000);
        assert!(level(&gone) < 0.001, "aliased: {}", level(&gone));
        // Up: the tone is the same tone at the higher rate.
        let up = resample(&tone(1_000.0, 16_000, 0.5), 1, 16_000, 48_000);
        assert!((level(&up) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        let want = tone(1_000.0, 48_000, 0.5);
        let worst =
            up.iter().zip(&want).skip(2_000).take(8_000).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 0.01, "{worst}");
    }

    #[test]
    fn channels_average_and_a_steady_level_stays() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        let steady = resample(&vec![0.25f32; 4_410], 1, 44_100, 16_000);
        assert!(steady.iter().all(|v| (v - 0.25).abs() < 1e-4));
        // Stereo keeps its channels apart.
        let stereo: Vec<f32> = (0..2_000).flat_map(|_| [0.5f32, -0.5]).collect();
        let out = resample(&stereo, 2, 48_000, 24_000);
        assert!(out.as_chunks::<2>().0.iter().all(|[l, _]| (l - 0.5).abs() < 1e-4));
        assert!(out.as_chunks::<2>().0.iter().all(|[_, r]| (r + 0.5).abs() < 1e-4));
        assert_eq!(resample(&[1.0, 2.0], 1, 8_000, 8_000), vec![1.0, 2.0]);
    }
}
