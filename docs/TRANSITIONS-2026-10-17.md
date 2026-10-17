# Transition workflow — 17 October 2026

Release: 0.24.0-beta. Analysis remains cache version 12; this release changes the editor/workflow rather than beat detection or the audio renderer.

## Explicit timing

Choose a transition in the Transition tab. The summary names A (outgoing) and B (incoming), shows the overlap’s mix bars and elapsed seconds, and lists the source-file intervals that meet there. Source intervals use each clip’s active grid and beat ratio. At an arranged cut, the ending timestamp comes from the region to the left, not the next source section. Split logical songs contribute all applicable fragments. Discontinuous intervals are listed separately.

The preset, per-stem modes, volume curves, overlap length and incoming beat ratio remain editable in the same view. Placeholder category/save controls were removed from this view. Individual stem choices require separated stems for both sides; the view explains the volume-curve fallback and offers separation.

## Processed monitoring

Set before/after bars, Repeat, and Play transition. Monitor A / Both / Monitor B switches monitoring without seeking or restarting the playhead. Both selected songs retain their transition envelopes, deck EQ/gain, automation, stems and FX processing. Deck mute/solo is overridden temporarily so existing monitor settings cannot silently suppress the selected side. Sources outside the selected logical pair are gated in the audition snapshot, keeping stable voice and transition indices.

This is processed monitoring. Shared master/live FX histories are retained when switching: a previous side’s reverb or echo tail may continue. The visible description and hover help explain this behavior. For dry attack alignment, the separate Compare view remains available.

The audition snapshot is separate from project/export data. Stop pauses audition and restores the exact previous loop range and loop-enabled state, then republishes normal playback state. Leaving the tab, choosing a different transition, invalidating its timing or reaching a one-shot endpoint stops audition. One-shot stopping is polled by the UI and is not a sample-accurate export-range command. Repeat uses the existing engine loop boundary handling. Starting a new audition reapplies the current before/after/Repeat choices; changing these controls during playback takes effect on the next start.

Project activation stops comparison/audition before clearing loop state, preventing a suspended loop from entering another document. Source relinking/timing changes invalidate the existing stamp. No new DSP engine, stretcher, worker or callback behavior is introduced.

## Saved protection

Protect this transition is persisted on the incoming logical transition, defaults to false for old projects, synchronizes across neutral split siblings, and supports Undo/Redo. It disables that transition’s preset, stem and volume-curve controls.

The first version uses a conservative placement rule: **any protected transition freezes arrangement moves, edge trims, cuts, section insertion/deletion, playlist reorder/Auto-order and automatic beat-ratio resolution**. This applies across the arrangement; it does not yet optimize unprotected songs around protected groups. Unlock the transition, or use Unlock all transitions, before editing placement. Playback and monitoring remain available. Master/FX processing, track automation and other transitions’ preset/stem/volume settings are separately editable; the lock is not an immutable rendered-audio seal.

Protection is checked in both editor paths and model layout, shift, edge, split, lane and section operations. The playlist’s existing track-position padlock remains a different control for Auto-order positions. Protection without a current overlap can be cleared using Unlock all transitions in the empty Transition view.

## Verification

- 140 ordinary tests passed; 9 opt-in local diagnostics ignored by default.
- All 122 release self-test checks passed. Model separation was skipped because its model is not downloaded.
- Release build and `cargo clippy --all-targets` completed successfully. Clippy retains existing warnings; this is not a warning-free lint claim.
- Isolated native demo startup and Transition view inspection passed at normal and minimum window sizes. The README screenshot uses generated demo sources.
- Independent read-only review identified existing mute/solo interference (reproduced and fixed) and shared master-FX-tail semantics (made explicit in the controls). A suspended-loop leak into new documents and missing model edge guard were reproduced and fixed before final verification.

The new regressions cover processed Both output against a normal two-track render with audible sources; outgoing volume-envelope retention in Monitor A; no Seek/Play command during side switches; no project/export mixer mutation; exact suspended-loop restoration; one-shot/stale-grid/tab exits; fresh-document loop cleanup; pre/post split fragments; source cuts with hand-derived intervals; persisted/undoable protection; legacy defaults; model guard paths; and essential controls inside the 1000-pixel-wide viewport.

These checks do not constitute physical listening certification on every backend, unrestricted long-project stress testing or grouping-aware locked Auto-order. Shared FX-tail continuity and arrangement-wide placement freezing are intentional limits of this version.
