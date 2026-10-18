# Beatgrid phase continuity — 2026-10-18

## Reproduced failure

Two local songs both had approximately 126 BPM and an audible sync problem in their overlap. In the saved outgoing Dynamic grid, a weak-drum passage drifted about 230 ms from the otherwise steady source clock before returning. The global BPM estimate was already close to correct. Dynamic-programming beat tracking had followed other rhythmic attacks, then local smoothing turned that excursion into artificial tempo motion.

A separate authored 126-BPM fixture reproduced a 237-ms grid error when onbeat kicks disappeared for a breakdown dominated by offbeat percussion. It failed before the fix. Fresh analysis of the local pair applied the correction, and the user reported the rendered overlap was clearly better. The ordinary overlap estimator still declined a correction because too few mutually consistent attacks were present; its selective measurements are not musical ground truth.

## Conservative correction

After attack refinement, analysis fits a steady source clock to a dominant, tight phase cluster. Acceptance requires at least 64 observations, 60% of observed attacks and 40% of predicted beats, at least 80% temporal span, and support in all three temporal thirds. The fitted period must remain within 0.3% of the coarse period. The original map is kept unless at least four consecutive markers show a phase excursion larger than 20% of a beat.

Real timing variation can occupy only part of a song and return to the original phase. A source-supported ramp veto prevents this correction when a sequence of reliable attacks corroborates that motion; bounded gaps and per-beat slopes accommodate halftime kicks. Existing small variations are left alone. Accepted corrections use the supported clock throughout the Dynamic map, avoiding a new discontinuity at the affected passage boundary. Flex remains an explicit, separate choice.

This is a guarded heuristic, not a general proof that a song has constant tempo or that its musical downbeat is correct. Ambiguous phase changes, unusual swing, weak or sparse material and tempo motion beyond the tested cases still require listening and manual editing. No audio-worker or stretcher changes are included.

## Adopting the result

Derived analysis cache is version 13. Saved project grids, source edits and arrangement placement are not automatically replaced. Beatgrid displays a source-qualified note when the correction was used. On an uncut source clip, **Reset to analysis** adopts the new result; listen through the transition and check the downbeat. Split/cut protections continue to apply. A regenerated cache is not itself a correction to a saved project.

## Verification

- 155 ordinary tests passed; 10 opt-in diagnostics ignored by default.
- The authored offbeat-breakdown regression failed with a 237-ms error before the correction and passed afterward. Negative controls cover sustained drift, tempo jumps, temporary/late changes, sparse halftime attacks and insufficient temporal support.
- Independent review found temporary- and sparse-tempo counterexamples. Both were reproduced as failing tests and protected before final verification; final scoped review reported no remaining must-fix issue.
- A widget regression confirms the source advisory displays without replacing a saved project grid.
- Six local files (six byte-distinct streams) were freshly analysed. Four available files from the earlier corpus retained identical estimated BPM, offset and static/Dynamic/Flex agreement diagnostics. Only the two newly reported sources used this correction. The sample is small, unannotated and not an accuracy benchmark.
- The user judged the corrected overlap clearly better. The final diagnostic WAV is byte-identical to that auditioned preview. The original project/recovery and audio sources were not overwritten.
- `cargo clippy --all-targets` completed with warnings; this is not a warning-free claim.

- Release build completed successfully. Isolated native demo startup and Beatgrid inspection passed at normal and minimum window sizes.

- All 122 release self-test checks passed. Model separation was skipped because its model is not downloaded.
