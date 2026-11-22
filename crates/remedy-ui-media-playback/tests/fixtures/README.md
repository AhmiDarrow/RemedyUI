# Test media

`clip.mp4`, `clip.vtt` and `clip.fr.srt` were made for these tests and are synthetic (solid
colors, a sine tone, two words). They are under the repository's license (MIT OR
Apache-2.0); there is no third-party content in them.

`clip.mp4` is 3 seconds, H.264 baseline 176x100 at 25 fps with a single key frame (so a seek
has to decode up to its target), video-range BT.709, red for the first second, green for the
second and blue for the third; and AAC-LC stereo at 48 kHz, a 440 Hz tone. It was made with:

```
ffmpeg -f lavfi -i "color=c=red:s=176x100:r=25:d=1" -f lavfi -i "color=c=lime:s=176x100:r=25:d=1" \
  -f lavfi -i "color=c=blue:s=176x100:r=25:d=1" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=3" \
  -filter_complex "[0:v][1:v][2:v]concat=n=3:v=1:a=0,format=yuv420p[v];[3:a]aformat=channel_layouts=stereo[a]" \
  -map "[v]" -map "[a]" -c:v libx264 -profile:v baseline -g 75 -keyint_min 75 -sc_threshold 0 -bf 0 \
  -colorspace bt709 -color_primaries bt709 -color_trc bt709 -color_range tv -c:a aac -b:a 64k \
  -movflags +faststart -map_metadata -1 -fflags +bitexact clip.mp4
```

`clip-subs.mp4` is `clip.mp4` with `clip.vtt` carried inside it as an English MPEG-4 timed-text
(`tx3g`) track:

```
ffmpeg -i clip.mp4 -i clip.vtt -map 0 -map 1 -c copy -c:s mov_text -metadata:s:s:0 language=eng \
  -movflags +faststart -map_metadata -1 -fflags +bitexact clip-subs.mp4
```

`clip-subs.mkv` is `clip.mp4`'s pictures and sound in Matroska with three text subtitle
tracks: `clip.vtt` as English SubRip (`S_TEXT/UTF8`), `clip.fr.srt` as French ASS
(`S_TEXT/ASS`), and `clip.vtt` again as WebVTT with no language (ffmpeg writes WebM's
`D_WEBVTT/SUBTITLES`). ffmpeg shifts the whole file 21 ms later, to the sound's first sample,
so its cues start at 0.521 s, 1.521 s and (the French) 0.021 s. Its cue index lists every
subtitle block:

```
ffmpeg -i clip.mp4 -i clip.vtt -i clip.fr.srt -i clip.vtt -map 0 -map 1 -map 2 -map 3 \
  -c:v copy -c:a copy -c:s:0 srt -c:s:1 ass -c:s:2 webvtt \
  -metadata:s:s:0 language=eng -metadata:s:s:1 language=fre \
  -map_metadata -1 -fflags +bitexact clip-subs.mkv
```

`clip-subs-streamed.mkv` is the same written to a pipe, as a live recording is: a segment of
unknown size and no cue index, so the reader walks it cluster by cluster:

```
ffmpeg -i clip-subs.mkv -map 0 -c copy -fflags +bitexact -f matroska pipe: | cat > clip-subs-streamed.mkv
```

`clip-subs.webm` is the clip as WebM (VP8 and Opus, re-encoded small) with `clip.vtt` as an
English WebVTT track, spread over three clusters:

```
ffmpeg -i clip.mp4 -i clip.vtt -map 0:v -map 0:a -map 1 -c:v libvpx -b:v 50k -c:a libopus -b:a 24k \
  -c:s webvtt -metadata:s:s:0 language=eng -map_metadata -1 -fflags +bitexact clip-subs.webm
```

`hls/` holds four small HLS streams cut from `clip.mp4` with ffmpeg 9.0.1: 1-second MPEG-TS
segments, a key frame every 25 frames so each segment starts on one. `C` below stands for
`-map_metadata -1 -fflags +bitexact -g 25 -keyint_min 25 -sc_threshold 0 -f hls -hls_time 1 -hls_flags independent_segments`.

`hls/aac`: H.264 High and AAC-LC (48 kHz stereo), 3 segments:

```
ffmpeg -i clip.mp4 -c:v libx264 -profile:v high -b:v 20k -c:a aac -b:a 24k $C \
  -hls_playlist_type vod -hls_segment_filename hls/aac/seg%05d.ts hls/aac/index.m3u8
```

`hls/mp3`: H.264 Constrained Baseline and MP3 (44.1 kHz stereo), the first 2 seconds:

```
ffmpeg -i clip.mp4 -t 2 -c:v libx264 -profile:v baseline -b:v 20k -c:a libmp3lame -b:a 32k -ar 44100 $C \
  -hls_playlist_type vod -hls_segment_filename hls/mp3/seg%05d.ts hls/mp3/index.m3u8
```

`hls/video`: H.264 Main without sound, an EVENT playlist, 3 segments:

```
ffmpeg -i clip.mp4 -an -c:v libx264 -profile:v main -b:v 20k $C \
  -hls_playlist_type event -hls_segment_filename hls/video/seg%05d.ts hls/video/index.m3u8
```

`hls/wrap`: as `hls/aac` but 2 seconds, with the stream's clock moved so far in (as Portal's
`-copyts` does, only further) that the 33-bit MPEG-TS clock wraps to zero 1.32 s in, inside the
second segment:

```
ffmpeg -copyts -i clip.mp4 -t 2 -c:v libx264 -profile:v high -b:v 20k -c:a aac -b:a 24k \
  -output_ts_offset 95441 $C -hls_playlist_type event \
  -hls_segment_filename hls/wrap/seg%05d.ts hls/wrap/index.m3u8
```

`hls/mono24` and `hls/surround` are sound-only streams of a 440 Hz tone, for the decoders'
handling of AAC formats. This ffmpeg has no HE-AAC encoder (no `libfdk_aac`; its `aac` and
`aac_mf` encoders make AAC-LC only), so there is no HE-AAC fixture. `hls/mono24` is AAC-LC at
24 kHz, mono, which Windows' AAC decoder takes for HE-AAC (twice the rate, stereo) until its
first frame says otherwise; `hls/surround` is AAC-LC 5.1 at 48 kHz:

```
ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=24000:duration=2" -c:a aac -b:a 16k \
  -map_metadata -1 -fflags +bitexact -f hls -hls_time 1 -hls_playlist_type vod \
  -hls_segment_filename hls/mono24/seg%05d.ts hls/mono24/index.m3u8
ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" -af "aformat=channel_layouts=5.1" \
  -c:a aac -b:a 48k -map_metadata -1 -fflags +bitexact -f hls -hls_time 1 -hls_playlist_type vod \
  -hls_segment_filename hls/surround/seg%05d.ts hls/surround/index.m3u8
```

`clip.h264`, `clip.aac` and `clip.mp3` are `clip.mp4`'s streams on their own, as an HLS segment
carries them, for the Windows HLS decoders' tests (`src/os/windows_hls.rs`): the pictures as an
Annex B byte stream, the sound as ADTS frames, and the sound re-encoded as MP3 at a constant
64 kb/s (192-byte frames, no ID3 or Xing header):

```
ffmpeg -i clip.mp4 -map 0:v -c copy -bsf:v h264_mp4toannexb -f h264 clip.h264
ffmpeg -i clip.mp4 -map 0:a -c copy -f adts clip.aac
ffmpeg -i clip.mp4 -map 0:a -c:a libmp3lame -b:a 64k -write_xing 0 -id3v2_version 0 \
  -map_metadata -1 -fflags +bitexact -f mp3 clip.mp3
```

`portal-frame.png` is the picture 20 seconds into the synthetic test pattern Portal's page is
compared with (colour bars, a moving line, a checkerboard; no third-party content), scaled to
480 wide, the picture `tests/portal_look.rs` plays. It was made with:

```
ffmpeg -ss 20 -i remedyui-cast-test.mp4 -frames:v 1 -vf scale=480:-2 -pix_fmt rgba portal-frame.png
```
