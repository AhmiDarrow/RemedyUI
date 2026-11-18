//! GPU work on the trace timeline: while something records GPU slices
//! ([`remedy_ui_trace::gpu_slices_wanted`]) and the device has timestamp queries, each frame's
//! GPU work is bracketed by two timestamps, read back once the GPU is done, and reported as
//! a slice of the measured GPU time. GPU and CPU clocks share no calibration in wgpu, so the
//! slice starts where the CPU submitted the work (the earliest the GPU can have started) and
//! lasts exactly as long as the GPU measured.
//!
//! Nothing is created or read while nobody records.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// The timestamps one frame writes: before its first pass and after its last.
const STAMPS: u32 = 2;
const STAMP_BYTES: u64 = 8;

const WAITING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

/// Frames whose timestamps are on their way back, per device.
#[derive(Clone, Default)]
pub(crate) struct GpuTimer(Arc<Mutex<Vec<Pending>>>);

impl std::fmt::Debug for GpuTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0.lock().map(|p| p.len()).unwrap_or(0);
        write!(f, "GpuTimer({n} pending)")
    }
}

struct Pending {
    submitted_ns: u64,
    period_ns: f32,
    readback: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

/// One frame's timestamp queries, between [`GpuTimer::begin`] and [`GpuTimer::submitted`].
pub(crate) struct Timing {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
}

impl GpuTimer {
    /// Writes the frame's first timestamp into `encoder`, when a trace wants GPU slices and
    /// `device` can take timestamps.
    pub(crate) fn begin(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) -> Option<Timing> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY)
            || !remedy_ui_trace::gpu_slices_wanted()
        {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("RemedyUI frame timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: STAMPS,
        });
        let size = u64::from(STAMPS) * STAMP_BYTES;
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RemedyUI timestamp resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RemedyUI timestamp readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        stamp(encoder, &queries, Some(0), None);
        Some(Timing { queries, resolve, readback })
    }

    /// Writes the frame's last timestamp and copies both where the CPU can read them.
    pub(crate) fn end(&self, timing: &Timing, encoder: &mut wgpu::CommandEncoder) {
        stamp(encoder, &timing.queries, None, Some(1));
        encoder.resolve_query_set(&timing.queries, 0..STAMPS, &timing.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &timing.resolve,
            0,
            &timing.readback,
            0,
            u64::from(STAMPS) * STAMP_BYTES,
        );
    }

    /// The frame was submitted at `submitted_ns` (trace clock): read its timestamps back when
    /// the GPU is done, and report what is ready from earlier frames.
    pub(crate) fn submitted(&self, timing: Timing, submitted_ns: u64, queue: &wgpu::Queue) {
        let state = Arc::new(AtomicU8::new(WAITING));
        let flag = state.clone();
        timing.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            flag.store(if r.is_ok() { READY } else { FAILED }, Ordering::SeqCst);
        });
        let pending = Pending {
            submitted_ns,
            period_ns: queue.get_timestamp_period(),
            readback: timing.readback,
            state,
        };
        if let Ok(mut p) = self.0.lock() {
            p.push(pending);
        }
        self.collect();
    }

    /// Reports every frame whose timestamps came back, and forgets those that failed.
    pub(crate) fn collect(&self) {
        let done: Vec<Pending> = match self.0.lock() {
            Ok(mut p) => {
                let (done, waiting): (Vec<Pending>, Vec<Pending>) =
                    p.drain(..).partition(|x| x.state.load(Ordering::SeqCst) != WAITING);
                *p = waiting;
                done
            }
            Err(_) => return,
        };
        for f in done {
            if f.state.load(Ordering::SeqCst) == READY {
                if let Some(ns) = measured(&f) {
                    remedy_ui_trace::gpu_slice("frame", f.submitted_ns, f.submitted_ns.saturating_add(ns));
                }
                f.readback.unmap();
            }
        }
    }
}

/// The GPU time between a frame's two timestamps, in nanoseconds.
fn measured(f: &Pending) -> Option<u64> {
    let view = f.readback.get_mapped_range(..).ok()?;
    let word = |i: usize| -> Option<u64> {
        let bytes: [u8; 8] = view.get(i * 8..i * 8 + 8)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    };
    let (a, b) = (word(0)?, word(1)?);
    drop(view);
    let ticks = b.checked_sub(a)?;
    // Ticks are whole periods; f64 keeps every tick count a frame can reach exact enough.
    let ns = (ticks as f64 * f64::from(f.period_ns)).round();
    (ns.is_finite() && ns >= 0.0).then_some(ns as u64)
}

/// An empty compute pass that only writes timestamps: a frame's work is bracketed without
/// touching the passes the renderer records.
fn stamp(encoder: &mut wgpu::CommandEncoder, queries: &wgpu::QuerySet, begin: Option<u32>, end: Option<u32>) {
    let pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("RemedyUI timestamp"),
        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
            query_set: queries,
            beginning_of_pass_write_index: begin,
            end_of_pass_write_index: end,
        }),
    });
    drop(pass);
}
