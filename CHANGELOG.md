# Changelog

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
