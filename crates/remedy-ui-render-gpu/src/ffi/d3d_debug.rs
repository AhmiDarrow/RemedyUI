//! `REMEDY_D3D_DEBUG=1`: the Direct3D debug layers for test runs (docs/d3d-debug-layers.md).
//!
//! The layers report through `OutputDebugString`, which raises a first-chance exception a
//! vectored handler sees with no debugger attached. wgpu's own handler only logs D3D12
//! messages, so this one catches D3D11, D3D12 and DXGI alike and ends the process on any
//! warning, error or corruption, after printing the message and where it was raised.

use std::sync::Once;

use windows::Win32::Foundation::{DBG_PRINTEXCEPTION_C, DBG_PRINTEXCEPTION_WIDE_C};
use windows::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
};

/// Whether the switch is set; read once per call site, so cheap enough for device creation.
pub(crate) fn enabled() -> bool {
    std::env::var_os("REMEDY_D3D_DEBUG").is_some_and(|v| v != "0")
}

/// Installs the handler (once per process).
pub(crate) fn install() {
    static ONCE: Once = Once::new();
    // SAFETY: the handler is a plain function that lives as long as the process.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(handler));
    });
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: Windows passes a valid record; for the two print exceptions the first two
    // parameters are the string's length (with its terminator) and its address.
    let message = unsafe {
        let r = &*(*info).ExceptionRecord;
        if r.NumberParameters < 2 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let (len, ptr) = (r.ExceptionInformation[0], r.ExceptionInformation[1]);
        match r.ExceptionCode {
            DBG_PRINTEXCEPTION_C => {
                String::from_utf8_lossy(std::slice::from_raw_parts(ptr as *const u8, len)).into_owned()
            }
            DBG_PRINTEXCEPTION_WIDE_C => {
                String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len))
            }
            _ => return EXCEPTION_CONTINUE_SEARCH,
        }
    };
    if is_failure(&message) && !(at_exit(&message) && crate::external::shared_made()) {
        let at = std::backtrace::Backtrace::force_capture();
        // Straight to stderr: the test harness's capture would be lost with the process.
        let line = format!("\nREMEDY_D3D_DEBUG: {}\n{at}\n", message.trim_end_matches(['\n', '\0']));
        let _ = std::io::Write::write_all(&mut std::io::stderr(), line.as_bytes());
        std::process::abort();
    }
    // Let wgpu's handler (or a debugger) have it too.
    EXCEPTION_CONTINUE_SEARCH
}

/// The layer's report of the objects alive when the process ends. Statics are never torn
/// down, so the process's shared device (`GpuContext::shared`) is alive then by design; any
/// other object alive then was leaked.
fn at_exit(message: &str) -> bool {
    ["D3D12 WARNING:", "DXGI WARNING:"]
        .iter()
        .find_map(|p| message.strip_prefix(p))
        .map(str::trim_start)
        .is_some_and(|m| m.starts_with("Process is terminating") || m.starts_with("Live "))
}

/// A debug-layer warning, error or corruption, less the layer's known false positives.
fn is_failure(message: &str) -> bool {
    let Some(rest) = ["D3D11 ", "D3D12 ", "DXGI "].iter().find_map(|p| message.strip_prefix(p)) else {
        return false;
    };
    if !["CORRUPTION:", "ERROR:", "WARNING:"].iter().any(|s| rest.starts_with(s)) {
        return false;
    }
    // The same two wgpu's handler drops: D3D12 #820/#821 ("no clear value given to resource
    // creation", a performance hint WebGPU cannot act on) and empty scissor rectangles, which
    // WebGPU allows. And #1361 (a comparison function on a non-comparison sampler, "This is
    // OK" in the layer's own words): wgpu-hal 30 fills `ComparisonFunc` with
    // `compare.unwrap_or_default()` (dx12/device.rs:777), so every plain sampler raises it.
    // And #1318 (a write-combined heap "can have a large performance impact" when the CPU
    // reads it): gpu-allocator 0.28 makes wgpu's upload heaps write-combined
    // (d3d12/mod.rs:290), which the CPU only writes.
    !(message.starts_with("D3D12 WARNING:")
        && ["#820:", "#821:", "DRAW_EMPTY_SCISSOR_RECTANGLE", "#1361:", "#1318:"]
            .iter()
            .any(|id| rest.contains(id)))
}

#[cfg(test)]
mod tests {
    use super::{at_exit, is_failure};

    #[test]
    fn the_exit_report_is_told_apart() {
        assert!(at_exit(
            "D3D12 WARNING: \tLive Object at 0x1, Refcount: 0. [ STATE_CREATION WARNING #0: UNKNOWN]"
        ));
        assert!(at_exit(
            "D3D12 WARNING: Live                         Object :    570 [ STATE_CREATION WARNING #0: UNKNOWN]"
        ));
        assert!(at_exit("D3D12 WARNING: Process is terminating. Using simple reporting."));
        assert!(at_exit("DXGI WARNING: Live Producer at 0x1, Refcount: 3. [ STATE_CREATION WARNING #0: ]"));
        assert!(!at_exit("D3D12 WARNING: ID3D12Device::CreateHeap: x"));
    }

    #[test]
    fn failures_are_warnings_and_worse_from_the_layers() {
        assert!(is_failure("D3D11 WARNING: x [ STATE_SETTING WARNING #1: Y]\n"));
        assert!(is_failure("D3D12 ERROR: x [ EXECUTION ERROR #538: Z]\n"));
        assert!(is_failure("DXGI ERROR: IDXGISwapChain::Present1: x\n"));
        assert!(is_failure("D3D11 CORRUPTION: x\n"));
        assert!(!is_failure("D3D12 INFO: x\n"));
        assert!(!is_failure("some other OutputDebugString\n"));
        assert!(!is_failure("D3D12 WARNING: x [ STATE_CREATION WARNING #820: CREATERESOURCE_CLEARVALUE]\n"));
    }
}
