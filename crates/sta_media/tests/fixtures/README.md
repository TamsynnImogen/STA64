# sta_media test fixtures

Synthetic clips made with the local ffmpeg CLI (libx264, native AAC encoder,
libaom). No game data. Regenerate with:

```sh
# h264_aac_160x120.mp4: 3 s, 25 fps, 75 frames, key frame every 25 with
# B-frames, AAC-LC 44.1 kHz stereo (27 KB)
ffmpeg -f lavfi -i "testsrc=size=160x120:rate=25:duration=3" \
  -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=3" \
  -c:v libx264 -profile:v main -preset fast -crf 30 -g 25 -bf 2 -pix_fmt yuv420p \
  -c:a aac -b:a 32k -ac 2 -shortest -movflags +faststart h264_aac_160x120.mp4

# h264_silent_96x64.mp4: 2 s, 30 fps, 60 frames, baseline, key frame every 15,
# no audio track (15 KB)
ffmpeg -f lavfi -i "testsrc2=size=96x64:rate=30:duration=2" \
  -c:v libx264 -profile:v baseline -preset fast -crf 30 -g 15 -pix_fmt yuv420p \
  -an h264_silent_96x64.mp4

# av1_64x64.mp4: 1 s AV1, only to check that an undecodable codec is reported
# as Unsupported (1.6 KB)
ffmpeg -f lavfi -i "testsrc=size=64x64:rate=10:duration=1" \
  -c:v libaom-av1 -cpu-used 8 -crf 40 -an av1_64x64.mp4
```

ffprobe / ffmpeg ground truth recorded in `tests/mp4.rs`: 75 frames and
132300 audio sample frames (`ffmpeg -f f32le`, 3.000 s exactly after the
encoder delay is removed) for the first clip. The decoded video of the first
clip is byte-identical to `ffmpeg -pix_fmt yuv420p` output and its audio
differs from ffmpeg's by at most 1.5e-5.
