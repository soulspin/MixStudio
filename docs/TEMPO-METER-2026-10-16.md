# Tempo-family and bar-phase verification — 2026-10-16

Release: 0.23.0-beta. Derived analysis cache: 12.

## Scope

Improve reproduced half/double-tempo failures and make unsupported bar-phase interpretations visible. Preserve the audio renderer, worker, transport, saved project grids and manual edits. The prior [corpus report](BEAT-ANALYSIS-2026-10-16.md) still describes the source material and its limits.

The global-tempo/DP structure is related to [Ellis’s beat-tracking method](https://www.ee.columbia.edu/~dpwe/pubs/Ellis07-beattrack.pdf). The new accent/coverage rules are local engineering heuristics; this release does not reproduce that paper’s evaluation or claim its results.

## Reference levels

1. **Authored rhythm references:** quarter-note timestamps and four-beat bar labels are specified before synthesis or analysis. Check 70, 80, 140, 180 and 190 BPM without added/skipped pulse markers inside the evaluated span; four pickup phases; normal and 1e-6 gain; loud backbeats; and strong eighth-note percussion at 70, 90 and 100 BPM. These references share one kick/accent model and do not represent every genre or instrumental timbre.
2. **Visually marked waveform pilot:** two local MP3 recordings, 15 attack landmarks in short windows around 60–64 seconds, marked from raw/low-band waveform plots before inspecting detected grids. Export windows with MixStudio’s decoder first: external decoders can handle MP3 delay differently. The manifest pins each source hash and decoded frame count. Use a predetermined 20 ms tolerance and require consecutive detected markers, so matching every other marker does not hide an octave error. These are attack landmarks, **not listening-verified musical beats, downbeats or tempo labels**.
3. **Unlabelled corpus regression:** the same ten files/eight byte-distinct streams as before. Related attack-envelope checks remain agreement proxies, not independent timing truth. Encoded variants and duplicates are not independent songs.

Private audio, paths, hashes and manifests remain local. No user recording is bundled or published.

## Reproduced failures and changes

| Controlled case | Earlier interpretation | Current requirement/result |
|---|---|---|
| Accented 70 BPM quarter notes | 140 BPM, intervening markers invented by range folding | 70 BPM, one marker per quarter note in the evaluated span |
| Accented 80 BPM quarter notes | Forced to the doubled family by the same range rule | 80 BPM |
| Accented 180/190 BPM quarter notes | Half-tempo family preferred | 180/190 BPM when a distinct four-pulse accent and attacks on every phase support it |
| Slow quarter notes plus strong eighth-note hats | 70 BPM could become 140 BPM | Separate bass-attack coverage retains 70/90/100 BPM in the authored controls |
| Supported 16–31-beat clip | No warning when bar-phase measurement was unavailable | Explicit source-qualified warning to check downbeat and half/double interpretation |

Remove the unconditional 85–175 fold. Retain the autocorrelation/comb estimate unless conservative evidence favors a related family. Doubling requires at least 32 pulses, a distinct four-pulse accent absent at the slower interpretation, and attacks across all four phases. Halving requires a distinct four-pulse bass accent with occupied phases, while the faster bass grid has nearly empty phases. The candidate range remains approximately 60–200 BPM; this is not an unrestricted tempo detector.

For bar-phase diagnostics, measure the selected phase’s relative bass-energy separation from the other three phases on the tracked grid. A small margin or unavailable measurement prompts listening and manual checking. This margin is gain-invariant on the controlled references, **not a calibrated confidence score or proof of the musical first beat**. A large bass accent can belong to another part of the bar. The existing downbeat selector is otherwise retained; this release does not claim to solve arbitrary musical bar labelling.

## Measured music results

All ten files retain exactly the same estimated global BPM, Dynamic beat count and Flex beat count as cache v11. Source hashes and decoded frame counts match. The new heuristics therefore do not establish an improvement in musical accuracy on this small unlabelled corpus; their demonstrated octave improvements come from the authored controls.

The waveform pilot also passes against the v11 baseline. It is additional independent reference coverage, not a measured timing improvement in this release.

| Anonymous source | Attack landmarks | v12 absolute median / sample p95 agreement |
|---|---:|---:|
| F | 6 | 6.0 / 6.7 ms |
| H | 9 | 7.2 / 10.8 ms |

Every pilot landmark is within 20 ms, with consecutive detected marker indices. The tiny samples, manual placement tolerance and selective windows preclude broad accuracy claims. None of these landmarks labels bar 1 or resolves 70-versus-140 by musical listening.

## Reproduction and compatibility

Ordinary reference tests live in `src/analysis/meter_tests.rs`. The read-only `inspect_manual_waveform_references` diagnostic accepts `MIXSTUDIO_REFERENCE_MANIFEST` (a private JSON array containing id, path, source_sha256, frames, window bounds, ordered attack_seconds and tolerance_seconds). `MIXSTUDIO_REFERENCE_EXPORT` exports decoder-aligned WAV windows without scoring. Without export mode, it scores the fixed annotations. `MIXSTUDIO_REFERENCE_OUTPUT` selects the local JSON report destination. Source hash, frame count, timing tolerance and consecutive-index checks are mandatory in scoring mode.

The existing corpus and 70-case negative-control diagnostics remain available. Derived cache v12 invalidates older results while retaining tags. The optional `downbeat_bass_margin` field defaults to unknown for older evidence; unknown evidence never becomes verified bar phase. Saved Track BPM/maps and arranged sections remain separate from refreshed source analysis. To adopt a new result, use Beatgrid → Reset to analysis on an uncut source clip and check it before section editing.

## Verification

- 131 ordinary tests passed; 9 opt-in diagnostics ignored by default. The final hats control also checks quarter-note timing within 10 ms and consecutive grid indices.
- All 122 release self-test checks passed, including rendered mixing, stretching/key lock, beat alignment, transport, section edits and saved-grid compatibility. Model separation was skipped because its model is not downloaded.
- Ten-file corpus diagnostic passed with unchanged BPM/offset, Dynamic/Flex counts and all grid-agreement distributions against v11.
- The fixed 15-landmark waveform pilot passed on v11 and v12 with identical results; all landmarks meet the predetermined 20 ms limit.
- Negative controls: zero supported Dynamic grids across 70 stationary noise/tone cases.
- `cargo clippy --all-targets` completed successfully with existing warnings; it is not warning-free. Release build and isolated native demo startup/Beatgrid visual inspection passed.
- Independent read-only code and documentation review found one short-clip uncertainty omission, reproduced and fixed before the final test run. No remaining concrete release blocker was found.

## Remaining limits

Strong syncopation, swing, live drummers, compound/triple metre and substantial tempo changes need independent annotated listening references. The current accent checks assume four pulses per bar. Equal pulses can support multiple musical interpretations, and weak/ambiguous material still needs manual correction. Flex remains experimental and opt-in. A larger listening-annotated corpus and calibrated uncertainty remain follow-up work.
