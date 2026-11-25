//! Linux: what the computer plays, from every program except this one, through the
//! PulseAudio client library (`libpulse.so.0`, opened at run time). PipeWire desktops answer
//! it too, through their PulseAudio server.
//!
//! Each other program's playback stream (a "sink input") is recorded on its own with a
//! monitor stream (`pa_stream_set_monitor_stream`), and the streams are mixed. Recording
//! the whole output's monitor instead would carry this program's own sound (a call's
//! voices) back out with a shared screen. Streams come and go as programs start and stop
//! playing; a subscription follows them.

use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use libloading::Library;

use crate::deps::{self, DependencyState};
use crate::host::{AudioFormat, AudioInput, MediaError};

type Ptr = *mut c_void;

const CONTEXT_READY: c_int = 4;
const CONTEXT_FAILED: c_int = 5;
const CONTEXT_TERMINATED: c_int = 6;
const STREAM_FAILED: c_int = 3;
const STREAM_TERMINATED: c_int = 4;
const SUBSCRIPTION_MASK_SINK_INPUT: c_uint = 0x0004;
const EVENT_FACILITY_MASK: c_uint = 0x000f;
const EVENT_SINK_INPUT: c_uint = 0x0002;
const EVENT_TYPE_MASK: c_uint = 0x0030;
const EVENT_NEW: c_uint = 0x0000;
const EVENT_CHANGE: c_uint = 0x0010;
const EVENT_REMOVE: c_uint = 0x0020;
const STREAM_DONT_MOVE: c_uint = 0x0200;
const STREAM_ADJUST_LATENCY: c_uint = 0x2000;
const SAMPLE_FLOAT32LE: c_int = 5;
/// `PA_CHANNELS_MAX`: the size of libpulse's channel arrays (its ABI, not a choice here).
const CHANNELS_MAX: usize = 32;
/// How often the server hands over sound.
const PERIOD: Duration = Duration::from_millis(10);

#[repr(C)]
#[derive(Clone, Copy)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

#[repr(C)]
struct ChannelMap {
    channels: u8,
    map: [c_int; CHANNELS_MAX],
}

#[repr(C)]
struct CVolume {
    channels: u8,
    values: [u32; CHANNELS_MAX],
}

/// The start of `pa_sink_input_info`, up to the fields read here.
#[repr(C)]
struct SinkInputInfo {
    index: u32,
    name: *const c_char,
    owner_module: u32,
    client: u32,
    sink: u32,
    sample_spec: SampleSpec,
    channel_map: ChannelMap,
    volume: CVolume,
    buffer_usec: u64,
    sink_usec: u64,
    resample_method: *const c_char,
    driver: *const c_char,
    mute: c_int,
    proplist: Ptr,
}

/// The start of `pa_sink_info`, up to the fields read here.
#[repr(C)]
struct SinkInfo {
    name: *const c_char,
    index: u32,
    description: *const c_char,
    sample_spec: SampleSpec,
    channel_map: ChannelMap,
    owner_module: u32,
    volume: CVolume,
    mute: c_int,
    monitor_source: u32,
}

type ContextCb = unsafe extern "C" fn(Ptr, Ptr);
type SubscribeCb = unsafe extern "C" fn(Ptr, c_uint, u32, Ptr);
type SinkInputCb = unsafe extern "C" fn(Ptr, *const SinkInputInfo, c_int, Ptr);
type SinkCb = unsafe extern "C" fn(Ptr, *const SinkInfo, c_int, Ptr);
type StreamRequestCb = unsafe extern "C" fn(Ptr, usize, Ptr);
type StreamCb = unsafe extern "C" fn(Ptr, Ptr);

/// The PulseAudio functions used.
struct Pa {
    mainloop_new: unsafe extern "C" fn() -> Ptr,
    mainloop_get_api: unsafe extern "C" fn(Ptr) -> Ptr,
    mainloop_start: unsafe extern "C" fn(Ptr) -> c_int,
    mainloop_stop: unsafe extern "C" fn(Ptr),
    mainloop_free: unsafe extern "C" fn(Ptr),
    mainloop_lock: unsafe extern "C" fn(Ptr),
    mainloop_unlock: unsafe extern "C" fn(Ptr),
    mainloop_wait: unsafe extern "C" fn(Ptr),
    mainloop_signal: unsafe extern "C" fn(Ptr, c_int),
    context_new: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    context_set_state_callback: unsafe extern "C" fn(Ptr, Option<ContextCb>, Ptr),
    context_connect: unsafe extern "C" fn(Ptr, *const c_char, c_uint, Ptr) -> c_int,
    context_get_state: unsafe extern "C" fn(Ptr) -> c_int,
    context_disconnect: unsafe extern "C" fn(Ptr),
    context_unref: unsafe extern "C" fn(Ptr),
    context_set_subscribe_callback: unsafe extern "C" fn(Ptr, Option<SubscribeCb>, Ptr),
    context_subscribe: unsafe extern "C" fn(Ptr, c_uint, Ptr, Ptr) -> Ptr,
    context_get_sink_input_info_list: unsafe extern "C" fn(Ptr, SinkInputCb, Ptr) -> Ptr,
    context_get_sink_input_info: unsafe extern "C" fn(Ptr, u32, SinkInputCb, Ptr) -> Ptr,
    context_get_sink_info_by_index: unsafe extern "C" fn(Ptr, u32, SinkCb, Ptr) -> Ptr,
    operation_unref: unsafe extern "C" fn(Ptr),
    stream_new: unsafe extern "C" fn(Ptr, *const c_char, *const SampleSpec, Ptr) -> Ptr,
    stream_set_monitor_stream: unsafe extern "C" fn(Ptr, u32) -> c_int,
    stream_set_read_callback: unsafe extern "C" fn(Ptr, Option<StreamRequestCb>, Ptr),
    stream_set_state_callback: unsafe extern "C" fn(Ptr, Option<StreamCb>, Ptr),
    stream_connect_record: unsafe extern "C" fn(Ptr, *const c_char, *const BufferAttr, c_uint) -> c_int,
    stream_get_state: unsafe extern "C" fn(Ptr) -> c_int,
    stream_peek: unsafe extern "C" fn(Ptr, *mut *const c_void, *mut usize) -> c_int,
    stream_drop: unsafe extern "C" fn(Ptr) -> c_int,
    stream_disconnect: unsafe extern "C" fn(Ptr) -> c_int,
    stream_unref: unsafe extern "C" fn(Ptr),
    proplist_gets: unsafe extern "C" fn(Ptr, *const c_char) -> *const c_char,
    _lib: Library,
}

// SAFETY: plain C function pointers into a library kept loaded for the process's life.
unsafe impl Send for Pa {}
// SAFETY: as above.
unsafe impl Sync for Pa {}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        // SAFETY: the symbol's type is the documented C signature of that libpulse function;
        // the library stays loaded while the table lives.
        *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("{}: {e}", $name))?
    };
}

impl Pa {
    fn load() -> Result<Pa, String> {
        // SAFETY: loading the PulseAudio client library, safe to load into any process.
        let lib = unsafe { Library::new("libpulse.so.0") }.map_err(|e| e.to_string())?;
        Ok(Pa {
            mainloop_new: sym!(lib, "pa_threaded_mainloop_new"),
            mainloop_get_api: sym!(lib, "pa_threaded_mainloop_get_api"),
            mainloop_start: sym!(lib, "pa_threaded_mainloop_start"),
            mainloop_stop: sym!(lib, "pa_threaded_mainloop_stop"),
            mainloop_free: sym!(lib, "pa_threaded_mainloop_free"),
            mainloop_lock: sym!(lib, "pa_threaded_mainloop_lock"),
            mainloop_unlock: sym!(lib, "pa_threaded_mainloop_unlock"),
            mainloop_wait: sym!(lib, "pa_threaded_mainloop_wait"),
            mainloop_signal: sym!(lib, "pa_threaded_mainloop_signal"),
            context_new: sym!(lib, "pa_context_new"),
            context_set_state_callback: sym!(lib, "pa_context_set_state_callback"),
            context_connect: sym!(lib, "pa_context_connect"),
            context_get_state: sym!(lib, "pa_context_get_state"),
            context_disconnect: sym!(lib, "pa_context_disconnect"),
            context_unref: sym!(lib, "pa_context_unref"),
            context_set_subscribe_callback: sym!(lib, "pa_context_set_subscribe_callback"),
            context_subscribe: sym!(lib, "pa_context_subscribe"),
            context_get_sink_input_info_list: sym!(lib, "pa_context_get_sink_input_info_list"),
            context_get_sink_input_info: sym!(lib, "pa_context_get_sink_input_info"),
            context_get_sink_info_by_index: sym!(lib, "pa_context_get_sink_info_by_index"),
            operation_unref: sym!(lib, "pa_operation_unref"),
            stream_new: sym!(lib, "pa_stream_new"),
            stream_set_monitor_stream: sym!(lib, "pa_stream_set_monitor_stream"),
            stream_set_read_callback: sym!(lib, "pa_stream_set_read_callback"),
            stream_set_state_callback: sym!(lib, "pa_stream_set_state_callback"),
            stream_connect_record: sym!(lib, "pa_stream_connect_record"),
            stream_get_state: sym!(lib, "pa_stream_get_state"),
            stream_peek: sym!(lib, "pa_stream_peek"),
            stream_drop: sym!(lib, "pa_stream_drop"),
            stream_disconnect: sym!(lib, "pa_stream_disconnect"),
            stream_unref: sym!(lib, "pa_stream_unref"),
            proplist_gets: sym!(lib, "pa_proplist_gets"),
            _lib: lib,
        })
    }
}

fn pa() -> Result<&'static Pa, MediaError> {
    static PA: OnceLock<Result<Pa, String>> = OnceLock::new();
    PA.get_or_init(Pa::load).as_ref().map_err(|_| {
        // The library could not be opened: it is not installed.
        let mut d = deps::libpulse("");
        d.state = DependencyState::NotInstalled;
        MediaError::Missing(d)
    })
}

/// One program's stream being recorded.
struct Recording {
    stream: Ptr,
    /// The output it plays on: a program moved to another output is recorded there anew.
    sink: u32,
    /// The callbacks' handle on this recording, freed with it.
    tag: *mut Tag,
    queue: VecDeque<f32>,
}

/// What the callbacks and the reader share; changed only while holding the main loop's lock.
struct Shared {
    mainloop: Ptr,
    context: Ptr,
    format: AudioFormat,
    own_pid: String,
    recordings: HashMap<u32, Recording>,
    failed: Option<String>,
}

// SAFETY: the pointers are libpulse objects, used only under the threaded main loop's lock.
unsafe impl Send for Shared {}

type State = Arc<Mutex<Shared>>;

/// A recording's handle for its callbacks: the shared state and which program it is.
struct Tag {
    state: *const Mutex<Shared>,
    input: u32,
}

fn lock(s: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The sound of every other program, mixed ([`crate::MediaHost::output_capture`]).
pub(crate) struct PulseCapture {
    pa: &'static Pa,
    state: State,
    /// The callbacks' pointer to `state`, released on drop.
    raw: *const Mutex<Shared>,
    last: Instant,
}

// SAFETY: libpulse calls go through the threaded main loop's lock.
unsafe impl Send for PulseCapture {}

unsafe extern "C" fn context_state(_c: Ptr, ud: Ptr) {
    // SAFETY: `ud` is the shared state's pointer, alive while the context is.
    let s = unsafe { &*ud.cast::<Mutex<Shared>>() };
    let Ok(pa) = pa() else { return };
    let mainloop = lock(s).mainloop;
    // SAFETY: waking whoever waits on the main loop (we are on its thread).
    unsafe { (pa.mainloop_signal)(mainloop, 0) };
}

unsafe extern "C" fn subscribed(c: Ptr, t: c_uint, idx: u32, ud: Ptr) {
    if t & EVENT_FACILITY_MASK != EVENT_SINK_INPUT {
        return;
    }
    let Ok(pa) = pa() else { return };
    // SAFETY: as in `context_state`.
    let s = unsafe { &*ud.cast::<Mutex<Shared>>() };
    let kind = t & EVENT_TYPE_MASK;
    if kind == EVENT_REMOVE {
        // A program that stopped playing.
        let gone = lock(s).recordings.remove(&idx);
        if let Some(r) = gone {
            end(pa, r);
        }
    }
    if kind == EVENT_NEW || kind == EVENT_CHANGE {
        // SAFETY: asking for the program's details; the operation is released at once.
        unsafe {
            let op = (pa.context_get_sink_input_info)(c, idx, sink_input, ud);
            if !op.is_null() {
                (pa.operation_unref)(op);
            }
        }
    }
}

/// Stops and frees one recording (on the main loop's thread, or under its lock).
fn end(pa: &Pa, r: Recording) {
    // SAFETY: the stream and its tag were made together and are released once.
    unsafe {
        (pa.stream_set_read_callback)(r.stream, None, std::ptr::null_mut());
        (pa.stream_set_state_callback)(r.stream, None, std::ptr::null_mut());
        (pa.stream_disconnect)(r.stream);
        (pa.stream_unref)(r.stream);
        drop(Box::from_raw(r.tag));
    }
}

/// A program is playing: unless it is this one, record it on its output's monitor.
unsafe extern "C" fn sink_input(c: Ptr, i: *const SinkInputInfo, eol: c_int, ud: Ptr) {
    if eol != 0 || i.is_null() {
        return;
    }
    let Ok(pa) = pa() else { return };
    // SAFETY: libpulse hands a valid info for the call's length; the state pointer is alive.
    let (info, s) = unsafe { (&*i, &*ud.cast::<Mutex<Shared>>()) };
    // SAFETY: reading a property of the info's property list (a C string or null).
    let pid = unsafe {
        let p = (pa.proplist_gets)(info.proplist, c"application.process.id".as_ptr());
        (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
    };
    {
        let mut g = lock(s);
        if pid.as_deref() == Some(g.own_pid.as_str()) {
            return;
        }
        match g.recordings.get(&info.index).map(|r| r.sink) {
            // Already recorded where it plays (a change of volume or title).
            Some(sink) if sink == info.sink => return,
            // Moved to another output: record it there instead.
            Some(_) => {
                if let Some(r) = g.recordings.remove(&info.index) {
                    drop(g);
                    end(pa, r);
                }
            }
            None => {}
        }
    }
    // The output's monitor source is where a monitor stream connects.
    let tag = Box::into_raw(Box::new(Tag { state: ud.cast(), input: info.index }));
    // SAFETY: asking for the output's details, with the tag as the callback's data (taken
    // back there).
    unsafe {
        let op = (pa.context_get_sink_info_by_index)(c, info.sink, sink_info, tag.cast());
        if op.is_null() {
            drop(Box::from_raw(tag));
        } else {
            (pa.operation_unref)(op);
        }
    }
}

unsafe extern "C" fn sink_info(c: Ptr, i: *const SinkInfo, eol: c_int, ud: Ptr) {
    // The callback runs once with the info and once more with `eol`; the tag goes on the last.
    let tag = ud.cast::<Tag>();
    if eol != 0 {
        // SAFETY: the tag was leaked for this request and is not used by a stream yet only if
        // no stream took it (a stream's tag is a separate box).
        unsafe { drop(Box::from_raw(tag)) };
        return;
    }
    if i.is_null() {
        return;
    }
    let Ok(pa) = pa() else { return };
    // SAFETY: valid for the call; the tag's state pointer is alive while the context is.
    let (info, t) = unsafe { (&*i, &*tag) };
    // SAFETY: as above.
    let s = unsafe { &*t.state };
    let (format, exists) = {
        let g = lock(s);
        (g.format, g.recordings.contains_key(&t.input))
    };
    if exists {
        return;
    }
    let spec = SampleSpec {
        format: SAMPLE_FLOAT32LE,
        rate: format.sample_rate,
        channels: format.channels.min(32) as u8,
    };
    let frag = (format.sample_rate as usize * usize::from(format.channels) * 4 / 100) as u32;
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: frag.max(64),
    };
    let device = std::ffi::CString::new(info.monitor_source.to_string()).unwrap_or_default();
    // SAFETY: a new record stream on the live context, monitoring the program's stream on its
    // output's monitor source; its callbacks get their own tag, freed when it ends.
    unsafe {
        let stream = (pa.stream_new)(c, c"Shared sound".as_ptr(), &spec, std::ptr::null_mut());
        if stream.is_null() {
            return;
        }
        let own = Box::into_raw(Box::new(Tag { state: t.state, input: t.input }));
        (pa.stream_set_monitor_stream)(stream, t.input);
        (pa.stream_set_read_callback)(stream, Some(readable), own.cast());
        (pa.stream_set_state_callback)(stream, Some(stream_state), own.cast());
        if (pa.stream_connect_record)(
            stream,
            device.as_ptr(),
            &attr,
            STREAM_DONT_MOVE | STREAM_ADJUST_LATENCY,
        ) < 0
        {
            (pa.stream_set_read_callback)(stream, None, std::ptr::null_mut());
            (pa.stream_set_state_callback)(stream, None, std::ptr::null_mut());
            (pa.stream_unref)(stream);
            drop(Box::from_raw(own));
            return;
        }
        lock(s)
            .recordings
            .insert(t.input, Recording { stream, sink: info.index, tag: own, queue: VecDeque::new() });
    }
}

unsafe extern "C" fn stream_state(st: Ptr, ud: Ptr) {
    let Ok(pa) = pa() else { return };
    // SAFETY: the stream is live for the call.
    let state = unsafe { (pa.stream_get_state)(st) };
    if state != STREAM_FAILED && state != STREAM_TERMINATED {
        return;
    }
    // SAFETY: the tag belongs to this stream and is alive until `end` frees it.
    let t = unsafe { &*ud.cast::<Tag>() };
    // SAFETY: as above.
    let s = unsafe { &*t.state };
    let gone = lock(s).recordings.remove(&t.input);
    if let Some(r) = gone {
        end(pa, r);
    }
}

unsafe extern "C" fn readable(st: Ptr, _n: usize, ud: Ptr) {
    let Ok(pa) = pa() else { return };
    // SAFETY: as in `stream_state`.
    let t = unsafe { &*ud.cast::<Tag>() };
    // SAFETY: as above.
    let s = unsafe { &*t.state };
    loop {
        let mut data: *const c_void = std::ptr::null();
        let mut len = 0usize;
        // SAFETY: peeking the stream's next fragment, then dropping it once read.
        if unsafe { (pa.stream_peek)(st, &mut data, &mut len) } < 0 || len == 0 {
            return;
        }
        let mut g = lock(s);
        // How much one program's stream may hold before its oldest is dropped: a reader that
        // stopped reading must not grow memory without end (`MediaLimits::capture_queue`).
        let cap = {
            let f = g.format;
            let held = crate::media_limits().capture_queue.as_millis() as usize;
            (f.sample_rate as usize * usize::from(f.channels)).saturating_mul(held) / 1000
        };
        if let Some(r) = g.recordings.get_mut(&t.input) {
            if data.is_null() {
                // A hole: that much silence.
                r.queue.extend(std::iter::repeat_n(0.0, len / 4));
            } else {
                // SAFETY: libpulse gives `len` readable bytes of 32-bit floats.
                let samples = unsafe { std::slice::from_raw_parts(data.cast::<f32>(), len / 4) };
                r.queue.extend(samples);
            }
            let over = r.queue.len().saturating_sub(cap);
            r.queue.drain(..over);
        }
        drop(g);
        // SAFETY: the fragment peeked above.
        unsafe { (pa.stream_drop)(st) };
    }
}

impl PulseCapture {
    pub(crate) fn open(format: AudioFormat) -> Result<PulseCapture, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 || format.channels > 32 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        let pa = pa()?;
        let state: State = Arc::new(Mutex::new(Shared {
            mainloop: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            format,
            own_pid: std::process::id().to_string(),
            recordings: HashMap::new(),
            failed: None,
        }));
        // The client's name in the system's mixer: this app's (libpulse copies it).
        let name = std::ffi::CString::new(super::app_name()).unwrap_or_else(|_| c"RemedyUI".to_owned());
        let raw = Arc::into_raw(state.clone());
        let ud = raw.cast_mut().cast::<c_void>();
        // SAFETY: the libpulse threaded main loop protocol: objects are made, the loop started,
        // and everything after happens under its lock; failures release what was made.
        unsafe {
            let ml = (pa.mainloop_new)();
            if ml.is_null() {
                drop(Arc::from_raw(raw));
                return Err(MediaError::Device("PulseAudio's main loop could not start".into()));
            }
            let ctx = (pa.context_new)((pa.mainloop_get_api)(ml), name.as_ptr());
            {
                let mut g = lock(&state);
                g.mainloop = ml;
                g.context = ctx;
            }
            let cap = PulseCapture { pa, state: state.clone(), raw, last: Instant::now() };
            if ctx.is_null() {
                return Err(MediaError::Device("no PulseAudio context".into()));
            }
            (pa.context_set_state_callback)(ctx, Some(context_state), ud);
            if (pa.context_connect)(ctx, std::ptr::null(), 0, std::ptr::null_mut()) < 0
                || (pa.mainloop_start)(ml) < 0
            {
                return Err(MediaError::Missing(deps::sound_server("no sound server answered")));
            }
            (pa.mainloop_lock)(ml);
            let ready = loop {
                match (pa.context_get_state)(ctx) {
                    CONTEXT_READY => break true,
                    CONTEXT_FAILED | CONTEXT_TERMINATED => break false,
                    _ => (pa.mainloop_wait)(ml),
                }
            };
            if !ready {
                (pa.mainloop_unlock)(ml);
                return Err(MediaError::Missing(deps::sound_server(
                    "the sound server refused the connection",
                )));
            }
            (pa.context_set_subscribe_callback)(ctx, Some(subscribed), ud);
            for op in [
                (pa.context_subscribe)(
                    ctx,
                    SUBSCRIPTION_MASK_SINK_INPUT,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ),
                (pa.context_get_sink_input_info_list)(ctx, sink_input, ud),
            ] {
                if !op.is_null() {
                    (pa.operation_unref)(op);
                }
            }
            (pa.mainloop_unlock)(ml);
            Ok(cap)
        }
    }
}

/// Mixes `frames` frames from each queue (silence where one has less) into `into`.
fn mix(queues: &mut [&mut VecDeque<f32>], samples: usize, into: &mut Vec<f32>) {
    let start = into.len();
    into.resize(start + samples, 0.0);
    let out = into.get_mut(start..).unwrap_or_default();
    for q in queues.iter_mut() {
        let n = q.len().min(samples);
        for (o, v) in out.iter_mut().zip(q.drain(..n)) {
            *o += v;
        }
    }
    for o in out.iter_mut() {
        *o = o.clamp(-1.0, 1.0);
    }
}

impl AudioInput for PulseCapture {
    fn format(&self) -> AudioFormat {
        lock(&self.state).format
    }

    fn period(&self) -> Duration {
        PERIOD
    }

    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError> {
        let (ml, ctx) = {
            let g = lock(&self.state);
            if let Some(e) = &g.failed {
                return Err(MediaError::Device(e.clone()));
            }
            (g.mainloop, g.context)
        };
        // SAFETY: reading the context's state under the main loop's lock.
        let alive = unsafe {
            (self.pa.mainloop_lock)(ml);
            let st = (self.pa.context_get_state)(ctx);
            (self.pa.mainloop_unlock)(ml);
            st == CONTEXT_READY
        };
        if !alive {
            let why = "the sound server went away".to_owned();
            lock(&self.state).failed = Some(why.clone());
            return Err(MediaError::Device(why));
        }
        // As much sound as time has passed: programs that play nothing add silence, so the
        // share's sound keeps time with its picture.
        let now = Instant::now();
        let mut g = lock(&self.state);
        let f = g.format;
        let frames =
            (now.duration_since(self.last).as_micros() * u128::from(f.sample_rate) / 1_000_000) as usize;
        if frames == 0 {
            return Ok(0);
        }
        self.last = now;
        let mut queues: Vec<&mut VecDeque<f32>> = g.recordings.values_mut().map(|r| &mut r.queue).collect();
        mix(&mut queues, frames * usize::from(f.channels), into);
        Ok(frames)
    }
}

impl Drop for PulseCapture {
    fn drop(&mut self) {
        let (ml, ctx) = {
            let g = lock(&self.state);
            (g.mainloop, g.context)
        };
        // SAFETY: tearing down in libpulse's order: streams and the context under the lock,
        // then the loop stopped and everything freed; the callbacks' state pointer last.
        unsafe {
            if !ml.is_null() {
                (self.pa.mainloop_lock)(ml);
            }
            let all: Vec<Recording> = lock(&self.state).recordings.drain().map(|(_, r)| r).collect();
            for r in all {
                end(self.pa, r);
            }
            if !ctx.is_null() {
                (self.pa.context_set_subscribe_callback)(ctx, None, std::ptr::null_mut());
                (self.pa.context_set_state_callback)(ctx, None, std::ptr::null_mut());
                (self.pa.context_disconnect)(ctx);
                (self.pa.context_unref)(ctx);
            }
            if !ml.is_null() {
                (self.pa.mainloop_unlock)(ml);
                (self.pa.mainloop_stop)(ml);
                (self.pa.mainloop_free)(ml);
            }
            drop(Arc::from_raw(self.raw));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_mixed_and_a_short_one_is_padded_with_silence() {
        let mut a: VecDeque<f32> = [0.25, 0.25, 0.25, 0.25].into();
        let mut b: VecDeque<f32> = [0.5, 0.9].into();
        let mut out = vec![9.0];
        mix(&mut [&mut a, &mut b], 4, &mut out);
        assert_eq!(out, [9.0, 0.75, 1.0, 0.25, 0.25], "summed, clipped, padded, appended");
        assert!(a.is_empty() && b.is_empty());
    }

    #[test]
    fn the_structures_match_libpulse() {
        // Offsets libpulse's headers give on 64-bit Linux.
        assert_eq!(std::mem::offset_of!(SinkInputInfo, sink), 24);
        assert_eq!(std::mem::offset_of!(SinkInputInfo, mute), 336);
        assert_eq!(std::mem::offset_of!(SinkInputInfo, proplist), 344);
        assert_eq!(std::mem::offset_of!(SinkInfo, monitor_source), 308);
    }
}
