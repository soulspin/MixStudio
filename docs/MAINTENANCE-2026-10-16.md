# Audit maintenance in MixStudio 0.21

The prioritised P1/P2 findings from the [16 October audit](AUDIT-2026-10-16.md) are addressed in this maintenance release. The work covers renderer capacity/state, transport timing, split-transition semantics, phrase/layout timing and file/input safety. It adds no new mixing feature.

## Renderer and transport

- **Key lock and timeline effects:** prepared snapshots allocate DSP buffers for the actual track/effect count instead of silently falling back after 64 tracks or ignoring effects after 16 blocks. Preparation and reclamation run on the producer; the audio callback transfers prepared state. A bounded retirement queue reserves capacity before enqueueing commands. Unchanged snapshots retain their WSOLA and effect history.
- **Effect ownership:** persisted effect identities distinguish existing, new, removed and replaced blocks. Preset copies and committed live gestures receive fresh identities; a held XY gesture retains its identity as parameters change. Kind/target changes start fresh DSP history. Removed Echo history cannot reappear in a later effect.
- **Seeking:** reset deck biquads, delay buffers/positions, WSOLA and click history. The host-default device buffer introduced in 0.16.1 is retained.
- **Tempo and loops:** duration and playback use matching integration of tempo linear in beats, including crossings of the 20 BPM floor. Per-sample beat timing preserves loop overshoot. Loop wraps reanchor stretched voices; loops shorter than a synthesis hop replay a cached anchored grain by elapsed phase instead of regenerating grains for every sample. Effect tails continue across wraps.
- **Effect boundaries:** activation/release are based on sample time. Flanger and FilterSweep no longer acquire different pre-start history from different callback partitions.
- **Export extent:** include the latest effect end plus its half-beat release, while retaining the existing allowance for transition echoes. A block at beat 10 with length 8 and tail 64 exports through at least beat 82.5.

## Editing and arrangement

New splits carry logical transition identity. Incoming/outgoing fragments retain the entire transition interval, preset, custom curves and stem choices; audio applies each logical envelope once per voice. Repeated splits and save/load retain this information. Splitting exactly on an existing source discontinuity preserves its short smoothing fades.

Deck alternation, transition drags and overlap-length changes keep logical fragments together. An unrelated sample removal no longer assigns siblings to different decks. Auto tempo advances its reference once per logical song and retains the original dynamic sampling and stabilised average across a split when fragment settings stay alike. Selective intro/outro stabilisation leaves the middle dynamic. Grouped ratio changes retain relative fragment placement.

Phrase display, snapping and phrase-based layout use source-grid timing and arranged occurrences, including beat ratios, trims, repeats and deleted source ranges. Manual/ramp layout resolves ratios at final positions and remains stable on the following frame. If discrete automatic ratios cycle, only the conflicting ratios become explicit; the layout remains undoable.

Compare dry preview includes all fragments, and its waveforms use the active fragment's mapping. Alignment stamps include every fragment. Automated measurement/correction and two-anchor correction require unsplit clips, so a partial representative cannot create or apply a misleading report. Manual source-grid work should precede splitting/section arrangement.

Legacy split projects have no fragment identity. They are not regrouped heuristically. New metadata uses defaults so old unsplit projects continue to load.

## Project and source safety

- Validate lane/mode indices, finite positive tempos, beat ratios, map ordering/spacing, trims, source ranges, automation, effect parameters and cue/phrase data before project activation or recovery. Failed validation retains the current document; invalid saves/checkpoints cannot replace a good file. Legacy duplicate automation timestamps remain accepted and guarded. Overcrowded single-beat edits retain their old marker instead of panicking.
- Distinguish packet/decoder errors from clean EOF. Reject incomplete PCM WAV data and verify native FLAC integrity. Handle verified zero padding from the FLAC encoder before trimming to the declared sample count. Do not compare unreliable MP3/AAC/AIFF duration estimates as though they were exact PCM counts.
- Mark incomplete folder scans as partial. Retain prior metadata under failed subtrees while applying confirmed deletions elsewhere; found files remain usable.
- Check cached model size and SHA-256 before session creation. Corrupt cached bytes are preserved and reported rather than silently loaded or automatically overwritten.
- Use a fresh private temporary directory for MP3 input, with scoped cleanup of owned files. An existing adjacent `set.mixstudio-tmp.wav` remains untouched.

## Verification

Every reproduced audit defect has a regression covering its failure mode. Independent read-only reviews caught additional low-tempo, fragment-order/lane, section-boundary, late-effect and microloop cases; those were reproduced before correction and rechecked afterward.

The ordinary Rust suite passes **104 tests**, with **5 opt-in local/device diagnostics** excluded from the default run. The release CLI selftest passes all 122 executed checks; model separation is skipped because its model is not downloaded. Selective key-lock stabilisation retains the dynamic middle, with measured boundary steps 0.0018/0.0004. The release build and Clippy complete; Clippy retains 98 binary-target and 124 test-target warnings, so this is not a warning-free lint claim. The checks include actual rendered samples, pitch and terminal timeline position, not just allocated output length:

- A 65th loaded key-locked voice matches the reference render; the 17th effect is audible.
- Unchanged prepared snapshots retain audio; deleted effects cannot emit old audio on a silent timeline.
- Ramp/loop output matches across callback partitions at 44.1 and 48 kHz in isolated reviewer checks. Late-start Flanger/FilterSweep differences dropped from peaks around 0.021/0.026 to zero in those cases. A one-sample loop repeats the same anchored audio sample.
- Forward/backward tempo inverses crossing the floor were checked independently across 19,200 pairs. The 10→40 BPM curve over 16 beats takes 38.18070978 seconds and reaches beat 16.
- Neutral split-transition tests cover presets, custom curves, arranged source cuts, ratios, repeated splits, save/load and one-step app Undo. Source-cut sample differences dropped from 0.17 to below 1e-6 in the regression.
- Generated fixtures decode through WAV, FLAC, MP3 with/without Xing, AAC, Vorbis, AIFF and ALAC. Malformed projects/recovery, partial scans, corrupted model bytes and MP3 scratch ownership are exercised without changing user recordings.
- A read-only check of the remaining saved local pair rendered 38.776 seconds at 44.1 and 48 kHz. Static and repeatedly updated snapshots had sample difference zero at both rates; the largest measured render callback was 3.146 ms.
- A real device-clock test used generated demo tracks and production prepared snapshots: default device at 44.1 kHz, 4.033 seconds wall time, 4.111 seconds timeline progress (ratio 1.019, within the scheduling test's 5% tolerance), with no reported audio errors.

Synthetic performance checks rendered four seconds of 44.1 kHz stereo with WSOLA and 16 Echo effects in roughly 0.12–0.20 seconds CPU on the development machine. This is workload evidence, not a guarantee for arbitrary project sizes or hardware.

## Remaining work

The original audit's research and scale items remain separate: analysis confidence in noise, half/double and musical downbeat ambiguity, stereo waveform cancellation, streaming/large-file export, a consistent Engine DJ database/WAL snapshot and model-download concurrency. Compressed sources without reliable sample totals may conceal clean frame-boundary truncation. Existing source analysis accuracy figures still describe a controlled corpus.

The device test verifies scheduling, not a physical listening assessment or every audio backend. Other operating systems, devices and hostile-scale projects need their own checks. Lint warnings and dependency-vulnerability scanning are not represented as resolved by these correctness fixes.
