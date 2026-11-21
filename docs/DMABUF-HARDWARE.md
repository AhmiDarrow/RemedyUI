# Checking zero-copy video on Linux hardware

The Linux GPU video path (GStreamer DMA-BUFs imported into wgpu's Vulkan device; PR #29) was
proven on lavapipe with a stand-in decoder. This is how to prove it, and measure it, with a
real hardware decoder: run one script, send back one file.

## What the machine needs

- **Native Linux, not WSL2.** WSL2's Vulkan driver for NVIDIA cards (Mesa's `dzn`, on D3D12)
  and its VA-API driver (Mesa's `d3d12`) are not expected to share DMA-BUFs with each other.
  Under WSL the renderer does not announce itself and playback stays on the copy path. The
  script says so instead of failing silently.
- **A Vulkan driver with `VK_EXT_external_memory_dma_buf`**, and ideally
  `VK_EXT_image_drm_format_modifier` (hardware decoders write tiled layouts) and
  `VK_KHR_external_semaphore_fd`. Mesa's `anv` (Intel) and `radv` (AMD) have all three, and so
  does NVIDIA's proprietary driver.
- **A GStreamer decoder that hands over DMA-BUFs:**
  - Intel or AMD: GStreamer 1.24's `va` plugin (`vah264dec`, in `gstreamer1.0-plugins-bad`)
    with Mesa's or Intel's VA driver (`mesa-va-drivers`, `intel-media-va-driver`).
  - NVIDIA: `nvidia-vaapi-driver` (NVDEC behind VA-API), then `vah264dec` as above.
    `nvh264dec` hands over CUDA or GL memory, not DMA-BUFs, so it doesn't count.
  - Arm boards: a V4L2 stateless decoder (`v4l2slh264dec`).
- An H.264 encoder for the measurement clips: `x264enc` (`gstreamer1.0-plugins-ugly`),
  `openh264enc` or `vah264enc`.
- `vulkan-tools` (for `vulkaninfo`) and `vainfo` (`vainfo` package), so the probe can say
  what is there.

On Ubuntu or Debian with an Intel or AMD GPU:

```bash
sudo apt install vulkan-tools vainfo mesa-va-drivers gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav
```

## Running it

From the repository root:

```bash
tools/dmabuf-hardware-check.sh --probe
```

```bash
tools/dmabuf-hardware-check.sh
```

The first command only reports what the machine has. The second runs everything:
1. The probe again.
2. The proof (`hardware_decoder_pictures_match_pictures_in_cpu_memory`, an ignored test):
   `clip.mp4` through a seek, decoded once to CPU memory and once into the hardware decoder's
   own DMA-BUFs. Every picture has to arrive in DMA-BUFs at the same time, be drawn by the
   GPU renderer in place with the same colours, and never be mapped on the CPU.
3. The stand-in test, which should also pass.
4. `measure_a_clip` at 720p, 1080p and 2160p: the copy path against `REMEDY_BENCH_PATH=hardware`
   (the decoder's own DMA-BUFs, no stand-in), two runs each.

The release build takes a few minutes the first time. Everything goes to
`target/dmabuf-hardware/report.txt`; `proof.txt`, `stand-in.txt` and `vulkaninfo.txt` sit next
to it.

## What to send back

Send `target/dmabuf-hardware/report.txt`, plus `proof.txt` if the proof failed.

If the proof fails, its message says where the path stopped:
- **"this Vulkan device does not import DMA-BUFs"**: the driver lacks the extension, or
  `open_device` could not open the device with it.
- **"N of 87 pictures came in DMA-BUFs"**: GStreamer chose a decoder that hands over CPU
  memory. Run with `GST_DEBUG=decodebin:4,va*:4` to see which decoder was chosen and what
  caps it offered.
- **"the GPU shows the same colours" / "renderer recorded errors"**: the import or the
  hand-over barriers misbehave on this driver. These are what lavapipe could not check:
  tiled modifiers, the sync-file semaphore, and the foreign-queue barriers.

## Running it from a local Claude Code session

Start Claude Code in your clone on the Linux machine and paste:

> Run `tools/dmabuf-hardware-check.sh --probe` and tell me what is missing for the zero-copy
> video check described in `docs/DMABUF-HARDWARE.md`. Install what is missing only after I
> agree. Then run `tools/dmabuf-hardware-check.sh`. If the proof fails, find out why from
> `target/dmabuf-hardware/proof.txt` and the code in
> `crates/remedy-ui-render-gpu/src/ffi/dmabuf.rs` and
> `crates/remedy-ui-media-playback/src/os/gst_dmabuf.rs`, fix it if it is ours, and re-run.
> Finish with the report's numbers in a table: copy vs hardware, per resolution.
