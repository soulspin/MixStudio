# Clip fades, samples and stem envelopes — 2026-10-18

Version 0.28 extends the existing timeline editing flow. Beat analysis and analysis cache version 13 are unchanged.

## Clip fades

Select a song (full-track view, automation closed) or a sample. The square corner handles set fade lengths in mix beats. The round midpoint sets the independent fade-in/out shape; double-click restores linear. The drawn curve and shared playback/export renderer use the same power envelope, with zero shape exactly linear. Old projects default to `[0, 0]` and keep their existing fade lengths. Short source-splice protection remains separate.

## Samples

Select a song section, then hold Alt and drag it onto Sample 1/2. In stem view, select a range with Ctrl+drag first or Alt-drag an entire stem row. The drop preview retains the original grab offset; dropping elsewhere or Escape cancels without an undo entry. Alt-drag an existing sample duplicates it.

Extracted samples reference original audio and arranged source ranges rather than writing a new WAV. They retain the source grid, current explicit beat ratio, gain, source stem mutes and sliced automation, including per-stem envelopes. Stem-only samples mute all other stems; the original song remains untouched. As with the existing project format, source audio and the stem model/cache are not bundled.

Shift-click adds/removes sample clips from the selection. Shift-drag empty space in the sample lanes selects a rectangle across both lanes. Drag any selected sample to move the group. The earliest clip clamps at mix beat zero, preserving spacing. Multi-clip movement keeps each clip's lane. Undo/redo restores the whole movement in one step; reopening a project clears transient selection/drag state. Sample rows are taller to make the waveform and fade controls usable.

## Stem envelopes

Separate the source, then choose Track → Edit stem volumes, or four-stem view → A. Opening the editor from two-stem view expands it to four individually labeled rows. Click a row to add a volume point; drag to change time/volume; right-click a point to remove it. Track → Reset stem volumes resets all four curves to unity in one undo step. Z enlarges the main decks.

Four additional automation lanes store drums, bass, melody and vocals in arranged, track-relative beats, ranging from 0 to 100%. Empty lanes mean unity. Existing section insertion/deletion, repeated source ranges, splits, extraction, saving and undo retain these envelopes. They multiply existing stem mutes and transition stem processing; they do not replace EQ or the track's overall gain. Curves are applied in direct, beat-slice, polyphonic key-lock and short-loop grain paths. The engine remains shared by device playback and offline export.

Required stems are requested for newly loaded, already-loaded and relinked project sources. Isolated samples remain silent while their required stems are unavailable, rather than playing the full song. Export reports that stems must be ready if any source with stem edits is unavailable. Stem separation still uses the existing optional HT-Demucs workflow; this release adds no model or dependency.

## Verification

- Baseline: 159 ordinary tests passed, 11 opt-in diagnostics ignored.
- Added renderer/project tests for independent edge shapes, legacy defaults, source mapping across repeated spans, stable extraction ratios (including half/double and fractional ratios), atomic group movement and serialized stem isolation.
- Added real pointer-event timeline tests for curve dragging/one-step undo, vocal-only envelope editing, Alt-drag selection/drop offsets and cross-lane rectangle selection/group movement.
- Added synthetic separated-audio tests across key-lock, direct and beat-slice modes. Repeated audio receives different arranged envelopes, and device-style rendering matches export with curved edge fades enabled.
- Independent review found two issues: ratio reset during sample extraction, and skipped stem preparation for reused/relinked sources. Both were reproduced with failing regressions and corrected. Follow-up review found no further must-fix issue. An additional short-loop regression then exposed stale cached audio after live stem-envelope edits; tiny-loop cache invalidation now includes the four stem envelopes without resetting ordinary polyphonic history during envelope drags.

These synthetic tests verify editing and processing behavior; they do not certify neural stem-separation quality on arbitrary recordings. The optional model is unchanged.

- Final ordinary suite: 172 passed, 11 opt-in diagnostics ignored. Release build and `cargo clippy --all-targets` succeeded; Clippy retains warnings, including the stem timeline drawing helper's argument count.
- The final release binary passed all 122 self-test checks. Real neural model separation was skipped because its optional model is not downloaded; synthetic stem reconstruction and envelope processing were exercised.
