# Beat/BPM analysis tests in 0.22

## Corpus and method

Fresh v10 and v11 analyses were run against ten local audio files: ten paths, eight byte-distinct streams, with two exact duplicates. Some remaining streams are different encodings or versions of the same title, so this is not eight independently annotated songs. No audio, source paths or private corpus fingerprints are published here. The test harness is opt-in and can take an explicit file manifest. Two v10 baseline runs produced identical reports. Final source hashes, durations, sample rates and frame counts matched baseline.

Each file is inspected in early/middle/late thirds. Up to 120 candidate beats per third are compared with separately implemented bass and percussion attack checks. Conflicting bass/percussion observations are rejected. The detector and these checks have related envelope assumptions, so residuals are **agreement proxies**, not independent annotated timing errors. Supported subsets can change between versions; a smaller p95 alone does not establish better musical accuracy. Three 45-second windows also provide local tempo estimates.

## Measured changes

The table lists p95 absolute agreement residuals in milliseconds as early/middle/late. File IDs are anonymous; exact duplicates are excluded.

| Source | Format | Estimated BPM v10 → v11 | Dynamic p95 v10, ms | Dynamic p95 v11, ms |
|---|---|---|---|---|
| A | MP3 | 132.0708 → 132.0708 | 19.1/16.2/77.6 | 19.0/16.1/75.3 |
| B | WAV | 132.0508 → 132.0508 | 32.6/88.4/57.8 | 20.6/76.5/23.9 |
| C | MP3 | 125.0037 → 125.0024 | 10.9/15.2/12.1 | 11.0/15.6/12.9 |
| D | MP3 | 123.9994 → 124.0012 | 9.9/15.0/18.8 | 10.1/15.0/14.2 |
| E | MP3 | 125.1729 → 125.1729 | 16.6/15.6/22.0 | 16.8/15.8/22.3 |
| F | MP3 | 125.7029 → 125.7029 | 6.6/9.8/9.2 | 7.2/9.8/9.4 |
| G | WAV | 124.5769 → 124.5763 | 7.8/12.9/14.9 | 8.2/12.9/15.3 |
| H | MP3 | 137.0330 → 137.0330 | 14.6/12.9/15.9 | 12.7/13.7/12.2 |

All ten Dynamic maps were retained with unchanged beat counts. Several weak-part proxies improved, while some others worsened slightly (roughly one millisecond or less). Source B still has a large middle residual: the change does not solve that recording. Flex remained less dependable in this corpus and stays explicit opt-in.

The recordings also show why a fixed BPM is insufficient: one file's local estimates are 125.0406/125.3439/125.8003 BPM; another's are 136.5490/137.1135/137.6693. These are detector estimates, not manually verified tempo curves. Variable grids remain variable rather than being forced onto a falsely precise constant.

## Reproduced failures and fixes

- A single kick plus quiet noise generated 108 nominal grid markers; stationary noise generated 127; a sustained 55 Hz carrier generated 141. They are not evidence of those many rhythmic events.
- Require repeated onset evidence, adequate lag overlap and at least 16 supported attacks. Refinement now checks sustained envelope growth across multiple carrier cycles, not just a steep sample-local ripple. A relative strength floor uses the upper attack percentile rather than a single loud outlier. Missing/weak attacks remain interpolated between evidence.
- Very quiet periodic audio at 123.4 BPM previously fell back to 128 BPM because `log(1+x)` and an absolute DP normalisation floor lost the onset evidence. `ln_1p` and scale-relative normalisation preserve the rhythm. The new regression retains BPM within 0.01 and supported beat positions within 5 ms at a gain multiplier of 1e-6.
- Cache optional source evidence: raw onset correlation, observed/predicted counts and temporal-third support. Beatgrid warns when the Dynamic map lacks sufficient evidence or a third has sparse attack support. Raw correlation and `grid_supported` are **not confidence probabilities, beat-accuracy guarantees or musical downbeat labels**. Even a sustained carrier can have very high onset correlation and must be rejected by the other gates. Internal support can exceed the separate attack-check support on real music.

## Compatibility and verification

Analysis cache version is 11. Library migration retains tags, and recomputed source analysis remains separate from saved Track BPM/maps. Existing projects and manual grids are preserved. To adopt a fresh result on an uncut source, choose Beatgrid → Reset to analysis before arranging sections. Legacy caches without the optional evidence field remain deserializable as unknown evidence; stale versions are recomputed.

Regressions cover false evidence, gain scaling, fractional steady BPM, missing kicks, ramps, stereo polarity, sample rates, cache compatibility and the actual warning widget. The opt-in negative-control sweep covers eight noise seeds plus six sustained tones at 2/4/8/15/60 seconds (70 cases). It requires zero supported Dynamic grids and writes the complete diagnostic locally. The original playback, transport and source-cut tests remain release gates.

Final verification: 125 ordinary tests passed (7 opt-in tests ignored); all 122 release self-test checks passed. The real-file corpus diagnostic passed and the separate negative-control sweep rejected all 70 non-rhythmic fixtures. `cargo clippy --all-targets` completed successfully with existing warnings; it is not warning-free. Model separation was skipped because its model is not downloaded. No playback engine or audio-worker code changed in this release.

## Limits and next work

The correlation/contrast gates are conservative engineering checks validated on this small corpus and controlled fixtures. Strong syncopation, large tempo changes, live drummers, swing and other genres need wider validation. Weak material can be declined rather than assigned a misleading Dynamic grid; its retained BPM is provisional. Half/double tempo and musical first-beat ambiguity remain unresolved by these checks. A labelled reference corpus and calibrated uncertainty are needed before claiming broader musical accuracy.
