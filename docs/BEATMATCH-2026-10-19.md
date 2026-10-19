# Beatmatch investigation — 2026-10-19

## Reproduced placement regression

Version 0.29 anchored a library drop's visible file edge to the snapped timeline position. A noninteger source lead-in then left the analysed downbeat between mix beats; even an integer lead-in could change the bar phase relative to normally appended tracks. A controlled 120 BPM source with a 0.375-second lead-in dropped at beat 20 reproduced a downbeat at 20.75.

Version 0.29.1 anchors main-deck downbeats to the captured destination. Following deck anchors use the active snap setting, while unsnapped targets stay fractional. Sample lanes retain exact visible-edge placement. Near timeline zero, deck anchors advance by complete active-snap units as necessary to retain introductory audio; with snapping off, only the minimum shift is applied. The drag tooltip distinguishes a deck downbeat from a sample's clip start. Explicit resolved beat ratios, asynchronous loading and one-batch undo remain intact.

Existing projects are not automatically repositioned. Source beat detection, saved maps, the audio worker and the key-lock backend are unchanged in this patch.

## Source and playback investigation

A read-only two-track local reproduction also exposed Dynamic-map excursions: conservative percussion attacks on the incoming source stayed near a fixed clock while the saved Dynamic map moved roughly 100 ms away and returned. This is separate from the placement regression. A fixed-grid comparison and a separate bar-placement comparison sounded better to the user, but listening still found disagreement around nine seconds into the overlap. Neither comparison establishes a complete repair of the automatic detector.

The first listening clips used 16 kHz output; subsequent key-lock/varispeed comparisons use matching 44.1 kHz output. The difference in sample rate made the initial comparison unsuitable for isolating the key-lock backend. Selective paired-attack and envelope-agreement measurements are diagnostics, not annotated musical beat accuracy or proof that an entire blend is in sync. No backend/window tuning is shipped on this evidence.

The saved reproduction uses Auto tempo, gliding from approximately 133 to 126 BPM through the blend. Constant-tempo comparisons distinguish this intentional whole-mix tempo motion from relative track drift. Listening verification of the remaining source/transition issue is ongoing; this patch only claims the reproduced library-placement repair.

The opt-in `inspect_saved_overlap_without_mutating_project` diagnostic accepts `MIXSTUDIO_DIAGNOSTIC_SR` for matched-rate output and `MIXSTUDIO_DIAGNOSTIC_SOLOS` to render each deck separately. Local projects, original music and generated comparisons remain private.

## Verification

The deck lead-in regression failed before the repair (20.75 instead of 20). The timeline-zero regression failed before intro protection was added (anchor 0 instead of 4). Both pass after the repair, alongside fractional unsnapped placement and the existing sample/ratio/undo tests. The full suite passed 184 ordinary tests, with 12 opt-in diagnostics ignored. Release compilation and Clippy completed; pre-existing lint warnings remain.

The release binary passed all 122 CLI self-test checks. Optional neural stem separation was skipped because its model is not downloaded. Independent read-only review found no important remaining placement defect, including the timeline-zero correction and unsnapped target behavior.
