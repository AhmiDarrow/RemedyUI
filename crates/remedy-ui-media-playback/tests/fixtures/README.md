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
