# Direct3D debug layers (Windows test runs)

Set `REMEDY_D3D_DEBUG=1` to run the GPU renderer, its DXGI swapchain and the Media Foundation
decoder's D3D11 device under the Direct3D debug layers. It needs Windows Graphics Tools
(Settings › System › Optional features), which puts `d3d12SDKLayers.dll` and
`d3d11_3SDKLayers.dll` in System32.

```powershell
$env:CARGO_HOME="$HOME/.cargo"; $env:WGPU_BACKEND="dx12"; $env:REMEDY_D3D_DEBUG="1"
cargo test -p remedy-ui-render-gpu
cargo test -p remedy-ui-media-playback
cargo test --release -p remedy-ui --test windows_auto_gpu
```

With the switch set:

- wgpu's instance gets D3D12 validation, debug labels and GPU-based validation
  (`InstanceFlags::advanced_debugging`). `WGPU_VALIDATION`, `WGPU_GPU_BASED_VALIDATION=0` and
  the other `WGPU_*` flags still override it, in release builds too. GPU-based validation
  makes runs several times slower, so the present-hang watchdog allows 20 times longer.
- The decoder's D3D11 device is created with `D3D11_CREATE_DEVICE_DEBUG`.
- A vectored exception handler (`render-gpu/src/ffi/d3d_debug.rs`) reads every
  `OutputDebugString` from the layers. On any D3D11, D3D12 or DXGI warning, error or
  corruption, it prints the message and a backtrace to stderr and aborts the test process,
  so the test fails.

With the switch unset, nothing changes.

Messages not treated as failures, each a known false positive:

| Message | Why |
|---|---|
| D3D12 #820/#821, empty scissor rectangles | wgpu's own handler drops these too: a clear-value performance hint WebGPU cannot act on, and an empty scissor rectangle, which WebGPU allows |
| D3D12 #1361 `CREATE_SAMPLER_COMPARISON_FUNC_IGNORED` | wgpu-hal 30 fills `ComparisonFunc` with `compare.unwrap_or_default()` (`dx12/device.rs:777`), so every plain sampler raises it; the layer itself calls it OK |
| D3D12 #1318 `WRITE_COMBINE_PERFORMANCE_WARNING` | gpu-allocator 0.28 makes wgpu's upload heaps write-combined (`d3d12/mod.rs:290`); the CPU only writes them |
| D3D12/DXGI live-object report at exit, once `GpuContext::shared` was made | that device is held by a static for the whole process by design; without it, any object alive at exit is a leak and fails the run |
