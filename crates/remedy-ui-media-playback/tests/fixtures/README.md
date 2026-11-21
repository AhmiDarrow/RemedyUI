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
