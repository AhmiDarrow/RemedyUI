//! Linux debug builds: how many GStreamer elements (pipelines among them) are alive, as
//! GStreamer's own leaks tracer counts them, when the process runs under
//! `GST_TRACERS=leaks`. The leak tests add these to the platform objects they count, so an
//! element a drop or an error path left behind (a source never set to NULL, a reference not
//! let go) fails them even where no Rust value holds it.

use std::ffi::{CStr, c_char, c_uint};
use std::sync::{Mutex, OnceLock, PoisonError};

use libloading::Library;

use super::gstreamer::{GBoolean, Ptr, gst};

struct Fns {
    active_tracers: unsafe extern "C" fn() -> Ptr,
    emit_by_name: unsafe extern "C" fn(Ptr, *const c_char, ...),
    type_name_from_instance: unsafe extern "C" fn(Ptr) -> *const c_char,
    structure_get_value: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    structure_free: unsafe extern "C" fn(Ptr),
    value_list_get_size: unsafe extern "C" fn(Ptr) -> c_uint,
    value_list_get_value: unsafe extern "C" fn(Ptr, c_uint) -> Ptr,
    value_get_structure: unsafe extern "C" fn(Ptr) -> Ptr,
    structure_get_string: unsafe extern "C" fn(Ptr, *const c_char) -> *const c_char,
    type_from_name: unsafe extern "C" fn(*const c_char) -> usize,
    type_is_a: unsafe extern "C" fn(usize, usize) -> GBoolean,
    element_type: unsafe extern "C" fn() -> usize,
    object_unref: unsafe extern "C" fn(Ptr),
    list_free: unsafe extern "C" fn(Ptr),
    _libs: [Library; 3],
}

/// A GLib list cell.
#[repr(C)]
struct GList {
    data: Ptr,
    next: *mut GList,
    prev: *mut GList,
}

fn load() -> Option<Fns> {
    // SAFETY: the system's GStreamer and GLib, already loaded by `gst()`; each symbol has
    // its documented C signature.
    unsafe {
        let gst = Library::new("libgstreamer-1.0.so.0").ok()?;
        let gobject = Library::new("libgobject-2.0.so.0").ok()?;
        let glib = Library::new("libglib-2.0.so.0").ok()?;
        Some(Fns {
            active_tracers: *gst.get(b"gst_tracing_get_active_tracers\0").ok()?,
            emit_by_name: *gobject.get(b"g_signal_emit_by_name\0").ok()?,
            type_name_from_instance: *gobject.get(b"g_type_name_from_instance\0").ok()?,
            structure_get_value: *gst.get(b"gst_structure_get_value\0").ok()?,
            structure_free: *gst.get(b"gst_structure_free\0").ok()?,
            value_list_get_size: *gst.get(b"gst_value_list_get_size\0").ok()?,
            value_list_get_value: *gst.get(b"gst_value_list_get_value\0").ok()?,
            value_get_structure: *gst.get(b"gst_value_get_structure\0").ok()?,
            structure_get_string: *gst.get(b"gst_structure_get_string\0").ok()?,
            type_from_name: *gobject.get(b"g_type_from_name\0").ok()?,
            type_is_a: *gobject.get(b"g_type_is_a\0").ok()?,
            element_type: *gst.get(b"gst_element_get_type\0").ok()?,
            object_unref: *gobject.get(b"g_object_unref\0").ok()?,
            list_free: *glib.get(b"g_list_free\0").ok()?,
            _libs: [gst, gobject, glib],
        })
    }
}

/// The GStreamer elements alive, by the leaks tracer; 0 when it is not running (release
/// builds, or no `GST_TRACERS=leaks`).
pub(crate) fn traced_elements() -> usize {
    static FNS: OnceLock<Option<Fns>> = OnceLock::new();
    if !cfg!(debug_assertions) || !std::env::var("GST_TRACERS").is_ok_and(|t| t.contains("leaks")) {
        return 0;
    }
    // The tracer starts with GStreamer: started here, the first count is not of nothing.
    if gst().is_err() {
        return 0;
    }
    let Some(f) = FNS.get_or_init(load) else { return 0 };
    // Elements made less elements gone since tracking started (the first call). Read from the
    // tracer's checkpoints, which hold type names and addresses only: its `get-live-objects`
    // takes a reference to each object, which races another thread's last unref.
    static NET: Mutex<i64> = Mutex::new(0);
    let mut net = NET.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: the active tracers list holds a reference to each tracer, each let go once; the
    // leaks tracer's checkpoint action hands over a structure, freed once; the values read
    // from it are borrowed while it lives.
    unsafe {
        let list = (f.active_tracers)();
        let mut cell = list.cast::<GList>();
        while !cell.is_null() {
            let tracer = (*cell).data;
            let name = CStr::from_ptr((f.type_name_from_instance)(tracer));
            if name.to_bytes() == b"GstLeaksTracer" {
                static STARTED: std::sync::Once = std::sync::Once::new();
                STARTED.call_once(|| (f.emit_by_name)(tracer, c"activity-start-tracking".as_ptr()));
                let mut point: Ptr = std::ptr::null_mut();
                (f.emit_by_name)(tracer, c"activity-get-checkpoint".as_ptr(), &mut point);
                if !point.is_null() {
                    *net += elements(f, point, c"objects-created-list");
                    *net -= elements(f, point, c"objects-removed-list");
                    (f.structure_free)(point);
                }
            }
            (f.object_unref)(tracer);
            cell = (*cell).next;
        }
        (f.list_free)(list);
    }
    usize::try_from(*net).unwrap_or(0)
}

/// How many entries of the checkpoint `point`'s list `field` are of an element type.
///
/// # Safety
/// `point` is a live checkpoint structure of the leaks tracer.
unsafe fn elements(f: &Fns, point: Ptr, field: &CStr) -> i64 {
    // SAFETY: values borrowed from the live structure; type names are C strings.
    unsafe {
        let objects = (f.structure_get_value)(point, field.as_ptr());
        let n = if objects.is_null() { 0 } else { (f.value_list_get_size)(objects) };
        let mut count = 0;
        for i in 0..n {
            let s = (f.value_get_structure)((f.value_list_get_value)(objects, i));
            let name =
                if s.is_null() { s.cast() } else { (f.structure_get_string)(s, c"type-name".as_ptr()) };
            if !name.is_null() && (f.type_is_a)((f.type_from_name)(name), (f.element_type)()) != 0 {
                count += 1;
            }
        }
        count
    }
}
