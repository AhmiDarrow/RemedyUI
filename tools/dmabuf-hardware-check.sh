#!/usr/bin/env bash
# Checks the Linux zero-copy video path (GStreamer DMA-BUFs into Vulkan) on real hardware, and
# measures it against the copy path. See docs/DMABUF-HARDWARE.md for what it needs and what
# to send back. Run from the repository root:
#
#   tools/dmabuf-hardware-check.sh            # probe, prove, measure
#   tools/dmabuf-hardware-check.sh --probe    # only say what this machine has
#
# Everything it prints is also written to target/dmabuf-hardware/report.txt.
set -u

OUT=target/dmabuf-hardware
mkdir -p "$OUT"
REPORT="$OUT/report.txt"
: > "$REPORT"
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }
section() { say ""; say "== $*"; }
fail=0

section "Machine"
say "kernel: $(uname -r)"
grep -qi microsoft /proc/version 2>/dev/null && say "WSL: yes (WSL's Vulkan and VA drivers are not expected to share DMA-BUFs; see the doc)"
[ -r /etc/os-release ] && say "os: $(. /etc/os-release; echo "$PRETTY_NAME")"
say "render nodes: $(ls /dev/dri/renderD* 2>/dev/null | tr '\n' ' ' || true)"
say "gstreamer: $(gst-inspect-1.0 --version 2>/dev/null | sed -n 's/^GStreamer //p' | head -1)"

section "Vulkan"
if command -v vulkaninfo >/dev/null; then
  vulkaninfo 2>/dev/null > "$OUT/vulkaninfo.txt"
  say "devices: $(grep -E '^\s*deviceName' "$OUT/vulkaninfo.txt" | sed 's/.*= //' | sort -u | tr '\n' ';')"
  say "drivers: $(grep -E '^\s*driverInfo' "$OUT/vulkaninfo.txt" | sed 's/.*= //' | sort -u | tr '\n' ';')"
  for e in VK_EXT_external_memory_dma_buf VK_EXT_image_drm_format_modifier VK_EXT_queue_family_foreign \
           VK_KHR_external_semaphore_fd VK_KHR_incremental_present; do
    if grep -q "$e" "$OUT/vulkaninfo.txt"; then say "  $e: yes"; else say "  $e: NO"; fi
  done
  grep -q VK_EXT_external_memory_dma_buf "$OUT/vulkaninfo.txt" || { say "!! no DMA-BUF import: the renderer will not take pictures on the GPU"; fail=1; }
else
  say "!! vulkaninfo not found (Debian/Ubuntu: vulkan-tools)"; fail=1
fi

section "Decoders"
command -v vainfo >/dev/null && vainfo 2>&1 | grep -E "Driver version|VAProfileH264" | head -5 | while read -r l; do say "  vainfo: $l"; done
hw=""
for d in vah264dec vaapih264dec v4l2slh264dec v4l2h264dec nvh264dec; do
  if gst-inspect-1.0 "$d" >/dev/null 2>&1; then say "  $d: present"; hw="$hw $d"; fi
done
[ -z "$hw" ] && { say "!! no hardware H.264 decoder in GStreamer (VA-API: gstreamer1.0-plugins-bad's va plugin plus a VA driver)"; fail=1; }
say "  note: nvh264dec hands over CUDA or GL memory, not DMA-BUFs; on NVIDIA use nvidia-vaapi-driver behind vah264dec"

if [ "${1:-}" = "--probe" ]; then
  say ""; say "probe only; report in $REPORT"; exit "$fail"
fi
[ "$fail" = 1 ] && say "" && say "!! this machine lacks something above; running anyway to show where it stops"

section "Proof: the hardware decoder's own DMA-BUFs, imported and drawn (clip.mp4, through a seek)"
cargo test --release -p remedy-ui-media-playback --lib \
  os::gst_dmabuf::tests::hardware_decoder_pictures_match_pictures_in_cpu_memory -- --exact --include-ignored --nocapture \
  > "$OUT/proof.txt" 2>&1
proof=$?
grep -E "renderer on|imports NV12|decoder's pictures|copied through CPU|panicked|^0 of|pictures came|test result" "$OUT/proof.txt" | tee -a "$REPORT"
[ "$proof" = 0 ] && say "proof: PASSED" || { say "proof: FAILED (full output in $OUT/proof.txt)"; fail=1; }

section "Stand-in test (the one CI runs; should pass anywhere the renderer imports DMA-BUFs)"
cargo test --release -p remedy-ui-media-playback --lib \
  os::gst_dmabuf::tests::pictures_in_dma_bufs_match_pictures_in_cpu_memory -- --exact --nocapture > "$OUT/stand-in.txt" 2>&1 \
  && say "stand-in: PASSED" || { say "stand-in: FAILED (see $OUT/stand-in.txt)"; fail=1; }

section "Measurement clips"
enc=""
for e in vah264enc x264enc openh264enc; do gst-inspect-1.0 "$e" >/dev/null 2>&1 && { enc=$e; break; }; done
if [ -z "$enc" ]; then
  say "!! no H.264 encoder to make clips with (x264enc: gstreamer1.0-plugins-ugly); skipping the measurement"
  exit 1
fi
for h in 720 1080 2160; do
  w=$(( h * 16 / 9 ))
  clip="$OUT/clip-$h.mp4"
  [ -s "$clip" ] || gst-launch-1.0 -q videotestsrc num-buffers=300 pattern=ball \
    ! "video/x-raw,width=$w,height=$h,framerate=30/1,format=I420" ! "$enc" ! h264parse ! mp4mux \
    ! filesink location="$clip" >/dev/null 2>&1
  [ -s "$clip" ] && say "  $clip (${w}x$h, 300 pictures, $enc)" || say "  could not make $clip"
done

section "Measurement: copy path vs the hardware decoder's DMA-BUFs (release, two runs each)"
[ "$proof" = 0 ] || say "!! the proof failed: 'hardware' rows below are the copy path too (look for '0 in DMA-BUFs')"
for h in 720 1080 2160; do
  clip="$OUT/clip-$h.mp4"
  [ -s "$clip" ] || continue
  for path in copy hardware copy hardware; do
    REMEDY_BENCH_CLIP="$PWD/$clip" REMEDY_BENCH_PATH=$path cargo test --release -p remedy-ui-media-playback --lib \
      os::gst_dmabuf::tests::measure_a_clip -- --exact --include-ignored --nocapture 2>&1 \
      | grep -E "pictures \(|panicked" | tee -a "$REPORT"
  done
done

say ""
say "done; report in $REPORT"
exit "$fail"
