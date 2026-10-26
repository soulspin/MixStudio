# Beatmatch and Library verification — 2026-10-26

## Reported problem

Two locally imported tracks at 125 and 126 BPM fell out of sync in a Constant 126 BPM mix, although both clips were stretched to the mix tempo.

## Root cause

The incoming track was correct. The outgoing track's Dynamic beat map wandered smoothly away from its kicks, up to about ±90 ms through the overlap, and its downbeat sat on the offbeat hi-hats (255 percussion attacks but 32 bass attacks on the old grid phase). The tracker skipped no beat, so the existing `phase::bridge` and `steady_tempo` safeguards, which search attacks near the tracker's own beats, followed the drift and did not engage. A straight clock at the analysed tempo matched the kicks throughout the file.

## Change

- `analysis::phase::steady_clock` searches bass attacks (combined attacks when bass is sparse) on a straight clock, independently of the tracker. It adopts the clock only when every third of the file supports it, at least 60 % of found attacks agree within 8 ms, the tracker departs from it, no run of eight off-clock attacks corroborates the tracker (genuine tempo motion), and the clock lands clearly more kicks within 10 ms than the tracker.
- `analysis::phase::kick_phase` moves the final grid half a beat, or by its median bass-attack offset, only when that lands at least 1.5× the kicks. Grids already on the kicks cannot qualify.
- Analysis cache version 14. Saved project grids are unchanged; Beatgrid offers **Use current analysis** for stale Dynamic clips. Reset keeps the downbeat on a whole beat in the previous bar phase.
- The grid-relative overlap measurement reports Uncertain when the outgoing track's own attacks drift more than 8 ms against its grid.

## Verification

- The reported pair, re-analysed read-only: overlap offset 1.7 ms and drift 0.07 ms, versus a drifting old grid now correctly reported as Uncertain.
- A read-only sweep of 303 local files (301 analysable): 72 grids changed and every one landed more kicks within 10 ms; 229 were unchanged. Total beats on the kick rose from 9,907 to 22,914. The adoption rule uses the same kick measure, so "never fewer" holds by construction; the size of the gains, not the absence of losses, is the evidence. Two drum & bass files remain unsupported, as before. File names and audio remain private.
- Regression tests cover a wandering and an offbeat tracker, an accurate tracker, a tempo jump and a late tempo change, offbeat and constant-offset grids, the grid-relative fallback with a drifting reference, Auto-align undo/verification/nudge direction, Reset bar phase, Library keys and preview playback (rendered preview correlates with the unstretched source at the cue; playhead and loop are restored; leaving the Library stops it).
- `cargo test --release` passes; `--selftest` passes.

## Limits

Kick alignment is not bar or phrase certainty: listen and check bar 1 after adopting a new grid. Half/double tempo and musical downbeats still need a labelled reference corpus. Preview uses the file's own analysed tempo and the master output settings; it is not routed through decks, EQ or effects.
