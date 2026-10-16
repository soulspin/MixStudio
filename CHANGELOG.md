# Changelog

## 0.21.1-beta

- Give analysis and action status messages a separate row below navigation. Long messages truncate to the available width and show their full text on hover, preventing overlap with menu and export buttons.

## 0.21.0-beta

- Resolve the prioritised renderer/editor/storage audit findings; record changes, regressions and remaining limits in docs/MAINTENANCE-2026-10-16.md.
- Prepare and retire variable-sized WSOLA/FX state outside realtime callbacks; remove hidden voice/effect caps, persist effect identity, preserve unchanged histories and reset deleted/replaced state. Retain the host-default audio buffer.
- Integrate tempo exactly, including the 20 BPM floor; preserve loop time and stretched-audio phase, reset all seek history, and process transition/FX timing per sample.
- Preserve logical split transitions, ratios, source-cut smoothing and Auto tempo; map phrases through source grids/sections, stabilise Manual/ramp layout and move logical groups coherently. Compare includes split fragments and protects unsupported grouped corrections.
- Validate project/recovery semantics, guard overcrowded beat edits, detect trustworthy decode truncation/integrity failures, retain partial-scan metadata, verify cached model bytes, include FX export tails and isolate MP3 scratch ownership.

## 0.20.0-beta

- Derive precise steady BPM from actual sample-level attack observations with minimum count, span, coverage and residual gates. Remove integer-tempo rounding, interpolate unsupported interior beats between observed attacks, and decline dynamic maps with fewer than 16 supported attacks.
- Choose stereo rhythm projection using onset evidence with a conservative mid preference; retain opposite-polarity rhythm without cancelling centred kicks under a sustained wide pad. Keep key analysis unchanged.
- Guard autocorrelation lags for short high-rate audio, bump analysis cache to version 10, refresh old derived library values, and reject stale/cancelled library analysis events.
- Fix duplicate automation point insertion and invalid drag bounds, preserve explicit split ratios, resolve Auto-tempo reorders against the requested order, and parse arbitrary UTF-8 keys safely.
- Preserve byte-backed drops in unique persistent source paths with exclusive writes instead of overwriting same-named temporary audio.
- Add synthetic ground-truth, stereo, sparse/ramp, library migration/race, edit and file-safety regressions; document the substantial audit and open priorities.

## 0.19.0-beta

- Keep independent background recovery snapshots of changed projects, offer recoverable copies at startup, and require Save as after restoration. Provide recovery status, explicit discard, and corruption/version handling.
- Save projects through a synced temporary sibling and atomic replacement, retaining the previous version as a .bak file. Reject invalid snapshots and enforce the same 64 MiB recovery limit on writes and reads.
- Wait for confirmed recovery writes before switching away from unsaved work or closing normally; keep the app/project open on protection failure. Recheck edits made while a checkpoint was in flight.
- Reset document-specific drag/edit state and save destinations at project boundaries; reject stale Save As, export, audio-picker, analysis, sample-render and relink callbacks.
- Relink all occurrences of moved source audio after explicit recording confirmation, retain grids/sections/cue/phrase data, reject insufficient source length and conflicting metadata, and undo relinks in one step.
- Add Ctrl+S / Ctrl+Shift+S, recovery demo view, and storage, throttling, failure, shutdown, project-boundary and relink regressions.

## 0.18.0-beta

- Combine bass attack evidence with broadband percussion onsets, reject conflicting timing, and expose weak versus supported sampled beats in the transition comparison view.
- Add aligned outgoing/incoming waveforms, 2–32 beat zoom, navigation, source-time anchor picking and a preview metronome.
- Audition outgoing, incoming or both tracks before transition processing. Preview is temporary, stops at the visible range end or normal pause, restores the exact suspended loop, and never alters export snapshots.
- Fit two manual source attacks to mix beats, preview the resulting BPM and audio/grid before explicit Apply, and undo the correction in one step. Preserve fractional anchors, active-map shape and internal map ratios. Protect edited sections and invalidate stale anchors after timing/audio changes.
- Reuse the proposed grid between playback snapshots to keep WSOLA identity stable; reflect current non-timing clip settings in preview.
- Add percussion, affine-fit, stale/Undo, waveform-click, loop restoration and preview/export isolation regressions.

## 0.17.0-beta

- Add a dedicated, scrollable Beatgrid tab with persistent whole-grid / following / single-beat modes, ±1/±10 ms corrections, beat navigation, zoom, downbeat, fixed/tap BPM and reset to analysis.
- Separate source analysis, active grid, mix tempo and beat ratio; simplify Track properties so controls remain reachable.
- Measure bass-attack alignment in transition overlaps on a background worker. Report offset, drift, scatter and usable beat count; enable an explicit constant correction only for consistent evidence. Reject silence, continuous bass, short/poor coverage and inconsistent attacks.
- Preserve neighbouring displayed beats for fractional-anchor single-beat edits, clamp markers inside neighbours, protect section arrangements, invalidate stale reports and undo corrections in one step. Targeted edits and measurements currently require a 1:1 beat ratio.
- Keep Dynamic as the default, retain the 0.16.1 playback buffer fix, and add detector, grid safety and headless interaction regressions.

## 0.16.1-beta

- Use the audio backend's default buffer instead of forcing a 512-frame buffer. On Linux/ALSA the forced buffer implied 128-frame periods (about 2.9 ms at 44.1 kHz), which produced an observed underrun with the native editor running. The backend's default provided a larger scheduling margin and passed the same native playback check without an underrun.
- Add optional saved-project diagnostics comparing offline rendering with live snapshot updates at 44.1/48 kHz, and checking audio-device clock progression. These checks read local audio and are ignored by the normal test suite.

## 0.16.0-beta

### Section editing

- Select a range on one waveform using the existing beat/bar snap settings.
- Copy and insert source sections at the playhead within the same song (Ctrl+C / Ctrl+V).
- Repeat the selected range directly after itself (Ctrl+D); the inserted copy remains selected.
- Delete a section and close the gap; ripple later songs while retaining their transition overlaps.
- Move later samples, effect blocks and manual tempo points with the edit.
- Audition a selection loop, clear it with Escape, and return to clip dragging with Move.
- Keep source audio untouched. Store arrangement edits in project JSON, preserve source automation and undo each edit in one step.

### Timing and interaction corrections

- Keep fractional source-grid offsets precise; restore compatible source BPM when returning to an analysed grid.
- Keep Dynamic as the default grid. Flex remains experimental and requires an explicit choice.
- Evaluate local tempo inside the active source section so repeats do not create artificial Auto-tempo jumps.
- Reset WSOLA at source splices, pre-roll inside the new section and apply short splice fades.
- Keep Beat Slice endpoints inside the current source section, including noninteger cuts.
- Preserve manual-tempo interpolation outside a rippled edit.
- Clear outdated selections after song, grid or placement changes.
- Allow an edited trim to reverse direction within the same drag gesture.

### Documentation and demos

- Refresh the README, shortcut reference, limitations and roadmap.
- Add current screenshots of section editing, transitions, beatgrid correction, automation/effects, playlist and overview.
- Add prepared demo views selected with MIXSTUDIO_DEMO_TAB.

### Validation

- Automated mapping, serialization, beatgrid, audio splice and headless interaction regressions.
- Full CLI selftest covering alignment, stretching, timeline editing, stems, effects, tempo, imports and export. Model separation is skipped when its model is unavailable.
- Optional saved-mix integration check using local source audio; no user audio is included in the repository.
