# Changelog

## Unreleased

Fixes from the [2026-10-27 audit](docs/AUDIT-2026-10-27.md), group 1 (crashes and data loss):

- A Single-beat edit at the editor's 5 ms limit no longer makes the project unsaveable or blocks closing; editor and validation share one minimum beat spacing (1).
- Edge fades are kept inside the clip after split, trim and section delete, and dragging a fade can no longer panic (2).
- XML imports nested deeper than 256 levels fail with a clear error instead of overflowing the stack and crash-looping at every start (3).
- NaN/inf samples are replaced at decode time, and analysis uses total ordering, so a damaged float file cannot panic analysis or leave loading unfinished (4).
- Delete, Undo and Redo end any in-progress drag, so a stale edge drag can no longer overwrite another track; the automation lane follows its track (9).
- A non-finite duration no longer empties the library index, and an unreadable library or index file is kept as `.unreadable-<time>` instead of being replaced (16).
- A decoder panic on a damaged file becomes a normal error in every loader (drop, preview, file load, Library analysis, relink) instead of "analysis timed out" or a load that never finishes (18).

Group 2 (regressions in 0.30):

- During a Library preview, edits (Ctrl+E, paste, nudge), snapping, ruler clicks and the transport buttons use the mix position. A click moves the parked mix playhead, which is kept when the preview stops; the preview itself is not moved. Transition and Compare auditions stop a running preview first (6, 7).
- The Details folder column compares whole folder names and the deepest music root, so `/Music/House2` is no longer shown as "2" under a `/Music/House` root (39).

Group 3 (sound quality):

- Key lock converts the source rate with a 16-tap windowed sinc instead of linear interpolation: 44.1 kHz tracks on a 48 kHz device keep their treble (14 kHz: −2.76 dB → −0.02 dB) and sound like the export (5).
- Loop wraps, seeks and resumes are declicked with ~3 ms fades; a 1-beat loop's wrap step fell from 0.79 to within the signal's normal slope (10).
- The deck low/high-pass filters keep running while bypassed, so re-engaging them no longer clicks (11).
- The metronome clicks only when the playhead crosses a beat start, not mid-beat after a seek or resume; loops click on every pass (26).
- Deck volume, mute/solo, A/B monitor switches and master gain ramp over ~5 ms instead of stepping (27).

Group 4 (split songs):

- Splitting inside a fade-in or fade-out is neutral: each half continues the same curve (`fade_skip` stores the part of a fade outside the clip), so the sound no longer jumps at the split (12).
- Set all overlaps, Overlap outros with intros, Auto-order and the playlist ⏶/⏷ lay out whole songs: a split song's halves move together, stay on one deck and keep their transitions. The playlist arrows use the chosen overlap length instead of 16 bars (13).
- Grid edits on one half of a split song (Beatgrid ±ms, `,`/`.`, Set downbeat, Reset/Use current analysis, grid mode, fixed BPM, beat-marker drags) apply to the whole song, so its halves stay contiguous and its transition unchanged (14).

## 0.30.0-beta

- **Beat analysis (cache v14):** search kicks on a straight clock directly and adopt it when it is supported throughout the source, the tracker departs from it, no run of attacks corroborates the departure and it lands clearly more kicks. A final check moves a grid half a beat or by its median kick offset only when that lands at least half as many kicks again. Fixes steady tracks drifting tens of ms off their kicks or sitting on offbeat hi-hats, which made 125/126 BPM imports fall out of sync.
- Beatgrid flags Dynamic clips whose saved grid differs from the current analysis and offers **Use current analysis**. Reset to analysis keeps the downbeat on a whole beat with the clip's previous bar phase.
- **Auto-align B** in the Transition, Beatgrid and Compare tabs: measure, apply a consistent offset or tempo-drift correction (two fitted anchors, at most ±2 %) as one Undo step, then measure again. Uncertain evidence is never applied.
- Measure outro/intro overlaps whose drums do not coincide against the shared mix grid (A up to 16 bars before, B up to 16 bars after). A drifting outgoing grid makes the result Uncertain.
- Nudge B by ±1/±10 ms in the Transition tab or with `,` / `.` (Shift for 10 ms). Beatmatch sits at the top of the Transition tab.
- **Library preview:** ▶ on any row plays the file alone at its original tempo from the first detected drop (else a third in), with a clickable waveform, position and stop. Space or a second click stops it; the mix playhead, loop and timeline view are restored. Never written to the project or export.
- **Library layout:** Library mode gives the browser most of the window with both decks visible; the Library tab opens larger; sizes are remembered separately. The folder list is a padded card with truncated names, right-aligned counts and a solid scrollbar. Search with icon and inline clear, framed filter toggles, arrow sort direction, analysis progress bar, airier rows with Camelot key chips and folders relative to the music root.
- **Enter** adds the selected Library files; **Ctrl+A** selects every visible file.
- Fix symbols that rendered as "□" and add a regression test that checks every UI string against the bundled fonts. Tidy panel styling with section headings, fact chips and hairline borders.
- Developer: `MIXSTUDIO_SCREENSHOT=out.png` saves the app's own frame and exits; `MIXSTUDIO_DEMO_TAB=beatmatch|library|listen` demonstrate the new features.

## 0.29.0-beta

- Redesign Library around readable filename-first rows, compact metadata columns, complete-path tooltips, details, search and filename sorting. Add a vertically resizable Library pane.
- Add indexed folder/subfolder browsing with counts and collapse advanced import/source controls. Cache folder structure independently from BPM/key updates; preserve Shift anchors by file identity.
- Drag whole rows to a visible snapped deck/sample destination, preserving list order and offset-aware positions. Decode and probe source caches in the background; apply each batch as one undoable edit with document/lock/error handling.
- Resolve beat ratios between batch appends so following songs use the correct predecessor length. Add pointer-event, mixed-tempo, selection, folder-boundary, failure and document regressions.

## 0.28.0-beta

- Add independent curved clip fade-in/out envelopes, direct length/shape handles and double-click linear reset; legacy shapes default to linear.
- Add non-destructive Alt-drag extraction of arranged song/stem sections, preserving source mapping, resolved beat ratios and automation. Support sample rectangle/Shift selection and atomic group movement.
- Add per-stem volume envelopes on four-stem rows, an explicit Track editor entry/reset, and shared playback/export processing across stretch modes. Preserve curves through section edits, copy, save and undo.
- Prepare required stems for reused and relinked project sources; block export until stem edits are ready and keep isolated samples silent while stems are unavailable.

## 0.27.0-beta

- Add a Transition harmony panel explaining whole-track Camelot relationships, musical key names, common matches and mixing tips. Treat unknown keys explicitly and distinguish source labels from measured overlap harmony.
- Add direct processed Both audition and pitch-following-tempo warnings across logical split fragments without changing saved mixer or arrangement settings.
- Evaluate the official Beat This! model on six local files and two authored references. Publish reproducible optional benchmark tooling and a source-hash-checked comparison; retain v13 analysis after the model reproduced an offbeat-breakdown phase failure.

## 0.26.0-beta

- Prevent reproduced half-beat Dynamic-grid excursions through offbeat breakdowns when broadly distributed source attacks support a steady clock. Preserve small timing variation and veto source-supported tempo motion, including sparse halftime attacks.
- Explain the source-analysis correction in Beatgrid. Upgrade derived cache to v13; retain saved grids and edits until explicitly reset.
- Add authored breakdown/tempo-motion regressions and opt-in saved-project phase diagnostics.

## 0.25.0-beta

- Add persisted, undoable per-song gain in Track and Transition; keep logical split fragments in sync.
- Replace the inactive auto-gain switch with a Master shortcut. Expose music gain in dB and the existing sample soft clipper’s bypass and ceiling. Legacy defaults preserve its transfer function.
- Measure whole-source stereo peak/RMS in the background; clearly distinguish RMS from LUFS and source estimates from processed output.
- Show held peaks before/after clipping and peak attenuation from device-consumed audio. Reset on the callback boundary without seeking; include the metronome in output clipping and metering.

## 0.24.0-beta

- Rework Transition into explicit source A/B intervals, overlap bars/duration, responsive preset/stem controls and processed audition. Respect arranged source cuts and logical split fragments.
- Add before/after bars, repeat and Monitor A/Both/B switching without seeking. Override monitor mute/solo only in playback snapshots; retain processed routing and shared FX tails. Restore suspended loop settings on stop and clear them on document changes.
- Persist undoable transition protection with legacy-safe defaults. Protected transitions freeze arrangement placement, trims/cuts/reorder and automatic ratios until unlocked, and protect their own preset/stem/volume curves.
- Add rendered-audio, command-order/transport, source-range, protection, document-lifecycle and minimum-width UI regressions.

## 0.23.0-beta

- Remove the unconditional 85–175 BPM fold. Compare supported pulse families using four-pulse accents and separate bass-attack coverage, reducing reproduced 70→140 and 180→90 octave errors.
- Add source-qualified bar-phase advisories for indistinct bass accents and short clips with insufficient evidence. Treat the optional measured bass margin as a heuristic, not confidence.
- Add authored tempo/metre references, a read-only evaluator for private waveform annotations and an actual warning-widget regression. Record independent-reference limits and before/after corpus preservation.
- Upgrade derived analysis cache to v12 while retaining saved project grids and library tags.

## 0.22.0-beta

- Test ten local audio files (eight byte-distinct streams) using reproducible fresh analysis and separately implemented attack-agreement proxies across temporal thirds. Record limits without claiming annotated ground truth.
- Gate dynamic grids on rhythmic repetition and sustained attack contrast; reject isolated/noise/carrier false evidence. Avoid unstable correlations from too few lag pairs.
- Use gain-safe `ln_1p` onset energy and relative normalisation for very quiet rhythmic audio. Refine using supported attacks while interpolating weak sections.
- Cache optional source rhythm evidence and show source-qualified insufficient/sparse-support advisories in Beatgrid. Upgrade analysis cache to v11 while preserving saved grids and library tags.


## 0.21.2-beta

- Use pinned Signalsmith Stretch for polyphonic key lock, with source-grid mapping, sample-rate conversion and explicit input/output latency compensation. Continue compatible logical fragments without restarting stretcher phase.
- Render into an eight-block pool on an audio worker; device callbacks copy prepared samples. Epoch changes flush stale audio on seek/pause/loop changes, while consumed sample clocks drive the playhead and meters. Count callback starvation for diagnostics.
- Match fallback WSOLA waveform similarity and transient energy per stereo channel. Opposite-polarity stereo content can no longer cancel out of the matching signal and suffer severe level loss during key-locked tempo changes.
- Cover bass and wide stereo stretching with sample-level regressions. Saved-project diagnostics can select a later overlap and exercise the production prepared-snapshot path.

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
