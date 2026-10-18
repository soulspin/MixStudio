# Beat This! pilot — 2026-10-18

## Protocol

Tested the official [CPJKU implementation](https://github.com/CPJKU/beat_this) at commit `b95c8ab0c58c2d9fcfd40508ae8dffbc05ac4f5c`, package 1.1.0, `final0`, minimal postprocessing (`dbn=False`). Both code and published weights are MIT licensed, subject to the upstream note about training-data rights. The checkpoint SHA256 was `8c328b45f59d8dd3dff219253ff6a8d6482be57d0133a29140e2febbf8eb8331`.

The isolated Python 3.12 environment used PyTorch 2.14.1+cpu and torchaudio 2.11.0+cpu, four CPU inference threads, FFmpeg decoding to 22,050-Hz stereo floats and the official mean-to-mono path. No GPU inference, Rust-port validation or modified-model inference is claimed. Model initialization took about 2.1 seconds; model execution on six local music files took 5.3–8.8 seconds each, excluding decoding/download/loading. Repeat runs while compiling the app took up to about 18 seconds per file. These are local measurements, not portable performance guarantees.

Two authored files test a 126-BPM clock with a 64-beat offbeat-percussion breakdown, and a gradual 120→128 BPM ramp. Their event times precede model inference. They are deliberately simple signals, not a representative music benchmark. Real music has no independently annotated beat/downbeat truth here.

## Findings

| Authored reference | MixStudio Dynamic median / P95 nearest-event error | Beat This! median / P95 nearest-event error | Reference beats within 25 ms, Dynamic / AI |
| --- | --- | --- | --- |
| Steady 126 BPM with offbeat breakdown | 0.3 / 0.3 ms | 7.6 / 238.1 ms | 100% / 75.4% |
| Gradual tempo ramp | 0.5 / 0.6 ms | 5.9 / 18.1 ms | 100% / 99.6% |

The common Rust diagnostic uses a percentile order statistic; the Python runner's interpolated P95 differs slightly before rounding. Nearest-event coverage does not penalize extra predictions and is not precision, recall or F1. Predicted Dynamic/AI beat counts were 258/254 for the breakdown and 258/256 for the ramp, against 256 authored beats in each case. AI predicted 199 and 158 downbeats respectively against 64 authored bar starts; nearest-downbeat coverage alone would hide these extra predictions. This pilot therefore cannot establish musical downbeat accuracy from the synthetic signals.

For the two reported FLAC sources, AI's 32-beat median tempo summaries were about 125.984 BPM. This summary is computed from 20-ms output positions; it is not an independently precise BPM estimate. Against the already corrected v13 steady grid, median nearest-beat agreement was approximately 7.8 and 5.8 ms. In source A's previously problematic 144–194-second region, P95 agreement was about 19 ms. **Agreement with our grid is not independent ground truth.**

AI's first downbeat was 0.06 seconds in both sources, while current source-analysis anchors were approximately 0.538 and 1.499 seconds. These disagree by roughly one and three beats; there is no annotated musical bar-phase reference to declare a winner. Some real-file predictions also contain missing-beat gaps or short intervals. Raw neural events must not be treated as a complete, validated playback map.

## Decision

Keep v13 as the automatic grid source. Beat This! is a promising separate hypothesis for pulse and bar phase, but this pilot does not justify replacing the tested clock with raw model events. A future integration needs gap handling, temporal validation, sample-level refinement and explicit comparison/adoption before touching saved grids. Model download/runtime packaging and cancellation/cache behavior are separate implementation work.

Version 0.27 adds the harmony panel; it does **not** bundle Beat This!, download weights on app startup, replace saved grids or change the stretcher/audio worker. No private music, model weights or local-path reports are published.

## Reproduction

`tools/benchmark_beat_this.py --manifest /absolute/path/to/private-paths.txt --output /absolute/path/to/private-results --synthetic` runs the optional Python benchmark after installing the official package and its dependencies in an isolated environment. The manifest contains one existing local source path per line. The model may download on first use. Generated fixtures use a fresh subdirectory; the manifest's audio is read only. Reports contain local paths and should be kept private.

For the same selective attack-agreement diagnostics and authored-reference error comparison as MixStudio:

```sh
MIXSTUDIO_EXTERNAL_BEATS=/absolute/path/to/private-results/results.json \
MIXSTUDIO_EXTERNAL_REPORT=/absolute/path/to/private-comparison.json \
cargo test compare_external_beat_predictions_without_mutating_sources -- --ignored --nocapture
```

The comparison validates source hashes before analysis and does not write project/source audio. Attack support is selective and must not be presented as annotated accuracy.
