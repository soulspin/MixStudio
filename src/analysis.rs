//! Decoding (symphonia), BPM/first-beat detection, key detection and 3-band waveform peaks.
use serde::{Deserialize, Serialize};
use std::path::Path;
use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};

#[cfg(test)]
#[path="analysis/diagnostics.rs"]
mod corpus_diagnostics;

#[cfg(test)]
mod grid_recommendation_tests {
    use super::*;

    #[test]
    fn tempo_jump_does_not_automatically_enable_experimental_flex() {
        let times = crate::testgen::jump_times(100.0, 64, 128.0, 96, 0.5);
        let (l, r) = crate::testgen::synth_kicks(&times, (*times.last().unwrap() + 1.0) as f32, 44100);
        let an = analyse(&Audio { sr: 44100, l, r }, "Jump".into(), String::new(), Path::new("jump.wav"));
        let saved = serde_json::to_value(&an).unwrap();
        assert_eq!(saved.get("recommended_grid_mode").and_then(|x| x.as_u64()), Some(1), "Flex must require an explicit choice until real-music validation exists");
        let map = an.flex_map().unwrap();
        for t in &times[2..times.len() - 2] {
            let error = map.beats.iter().map(|b| (b - t).abs()).fold(f64::MAX, f64::min);
            assert!(error < 0.010, "recommended grid missed a kick by {error:.3} s");
        }
    }

    #[test]
    fn steady_track_analysis_keeps_the_dynamic_grid() {
        let times: Vec<_> = (0..128).map(|i| 0.5 + i as f64 * 0.5).collect();
        let (l, r) = crate::testgen::synth_kicks(&times, 65.0, 44100);
        let an = analyse(&Audio { sr: 44100, l, r }, "Steady".into(), String::new(), Path::new("steady.wav"));
        let saved = serde_json::to_value(&an).unwrap();
        assert_eq!(saved.get("recommended_grid_mode").and_then(|x| x.as_u64()), Some(1));
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    #[test]
    fn fractional_tempo_does_not_accumulate_integer_rounding_drift() {
        for expected in [127.98, 128.025] {
            let sr = 22050;
            let times: Vec<_> = (0..768).map(|k| 0.37 + k as f64 * 60.0 / expected).collect();
            let (m, _) = crate::testgen::synth_kicks(&times, (times.last().unwrap() + 1.0) as f32, sr);
            let (bpm, _, beats) = detect_beats(&m, sr);
            let mut errors: Vec<_> = times[4..times.len()-4].iter().map(|t| beats.iter().map(|b| (b-t).abs()).fold(f64::INFINITY, f64::min)).collect();
            errors.sort_by(f64::total_cmp);
            let drift = (bpm / expected - 1.0).abs() * 360.0;
            eprintln!("precision expected={expected} bpm={bpm:.7} drift360={drift:.6}s median={:.6}s p95={:.6}s", errors[errors.len()/2], errors[errors.len()*95/100]);
            assert!(drift < 0.005, "fractional tempo drift: {drift:.6}s over six minutes ({bpm:.7} vs {expected})");
            assert!(errors[errors.len()*95/100] < 0.005, "beat p95 exceeds 5ms");
            let matched:Vec<_>=times[4..times.len()-4].iter().map(|t|beats.iter().enumerate().min_by(|a,b|(a.1-t).abs().total_cmp(&(b.1-t).abs())).unwrap().0).collect();
            assert!(matched.windows(2).all(|w|w[1]==w[0]+1),"grid skipped or added beats inside the evaluated span");
        }
    }

    #[test]
    fn opposite_polarity_stereo_retains_rhythm() {
        let sr = 22050;
        let times: Vec<_> = (0..96).map(|k| 0.37 + k as f64 * 60.0 / 123.4).collect();
        let (l, _) = crate::testgen::synth_kicks(&times, 48.0, sr);
        let r = l.iter().map(|x| -x).collect();
        let an = analyse(&Audio {sr, l, r}, "Polarity".into(), String::new(), Path::new("polarity.wav"));
        assert!((an.bpm - 123.4).abs() < 0.01, "lost stereo rhythm: {}", an.bpm);
        assert!(times[4..90].iter().all(|t| an.beats.iter().any(|b| (b-t).abs() < 0.005)), "lost beat positions");
    }

    #[test]
    fn wide_sustained_pad_cannot_hide_centred_kicks() {
        let sr=22050;
        let times:Vec<_>=(0..96).map(|k|0.37+k as f64*60.0/123.4).collect();
        let (kick,_)=crate::testgen::synth_kicks(&times,48.0,sr);
        let mut l=Vec::new(); let mut r=Vec::new();
        for (i,k) in kick.iter().enumerate() {
            let pad=0.5*(2.0*std::f32::consts::PI*440.0*i as f32/sr as f32).sin();
            l.push(k+pad); r.push(k-pad);
        }
        let (bpm,_,_)=detect_beats(&rhythm_mono(&Audio {sr,l,r}),sr);
        assert!((bpm-123.4).abs()<0.01,"sustained side channel stole tempo: {bpm}");
    }

    #[test]
    fn short_audio_at_normal_sample_rates_does_not_panic() {
        for sr in [22050, 44100, 48000, 96000] {
            let mut m = vec![0.0; sr as usize]; m[sr as usize/2]=0.5;
            let (bpm, offset, _) = detect_beats(&m, sr);
            assert!(bpm.is_finite() && offset.is_finite());
        }
    }

    #[test]
    fn beat_precision_across_sample_rates_tempos_and_missing_kicks() {
        for (sr,expected) in [(22050,90.25),(44100,123.4),(48000,128.025),(96000,170.3)] {
            let times:Vec<_>=(0..160).map(|k|3.37+k as f64*60.0/expected).collect();
            let audible:Vec<_>=times.iter().enumerate().filter(|(i,_)| !(48..64).contains(i) && i%13!=0).map(|(_,t)|*t).collect();
            let (m,_)=crate::testgen::synth_kicks(&audible,(times.last().unwrap()+1.0) as f32,sr);
            let (bpm,_,beats)=detect_beats(&m,sr);
            let mut errors:Vec<_>=times[4..156].iter().map(|t|beats.iter().map(|b|(b-t).abs()).fold(f64::INFINITY,f64::min)).collect();
            errors.sort_by(f64::total_cmp);
            eprintln!("matrix sr={sr} expected={expected} bpm={bpm:.6} p95={:.6}s",errors[errors.len()*95/100]);
            assert!((bpm-expected).abs()<0.01,"incorrect BPM {bpm} vs {expected}");
            assert!(errors[errors.len()*95/100]<0.005,"beat precision lost at {sr} Hz");
        }
    }

    #[test]
    fn steady_tempo_fit_rejects_a_deliberate_tempo_ramp() {
        let beats=crate::testgen::ramp_times(120.0,130.0,180.0,0.37);
        let observed:Vec<_>=beats.iter().enumerate().map(|(i,&t)|(i as f64,t)).collect();
        assert!(steady_tempo(&observed,125.0).is_none());
    }
    #[test]
    fn isolated_attack_is_not_evidence_for_a_steady_tempo_fit() {
        let (m,_)=crate::testgen::synth_kicks(&[0.37],60.0,22050);
        let coarse=detect_bpm(&m,22050).0;
        let (bpm,_,beats)=detect_beats(&m,22050);
        assert_eq!(bpm,coarse,"inferred silent beats changed BPM");
        assert!(beats.is_empty(),"one attack must not produce a trusted dynamic grid");
    }
    #[test]
    fn end_to_end_ramp_keeps_variable_timing_instead_of_claiming_a_steady_fit() {
        let times=crate::testgen::ramp_times(120.0,130.0,180.0,0.37);
        let (m,_)=crate::testgen::synth_kicks(&times,181.0,22050);
        let coarse=detect_bpm(&m,22050).0;
        let (bpm,_,beats)=detect_beats(&m,22050);
        assert_eq!(bpm,coarse,"variable attacks must not qualify for a steady fit");
        assert!(beats.windows(2).all(|w|w[1]>w[0]));
        let mut errors:Vec<_>=times[8..times.len()-8].iter().map(|t|beats.iter().map(|b|(b-t).abs()).fold(f64::INFINITY,f64::min)).collect();
        errors.sort_by(f64::total_cmp);
        assert!(errors[errors.len()*95/100]<0.010,"ramp beat p95 exceeded10ms");
    }

    #[test]
    fn periodicity_needs_more_than_a_few_overlapping_pairs() {
        let onset:Vec<f32>=(0..64).map(|i|if i%3==0 {1.0}else{0.0}).collect();
        assert_eq!(tempo_support(&onset,100.0,100.0),0.0,"three matching pairs do not establish repetition");
    }
    #[test]
    fn rhythm_evidence_survives_cache_and_legacy_cache_is_unknown() {
        let sr=22050;let times:Vec<_>=(0..128).map(|k|0.37+k as f64*0.5).collect();
        let (m,_)=crate::testgen::synth_kicks(&times,65.0,sr);
        let (_,_,beats,evidence)=detect_rhythm(&m,sr);
        assert!(evidence.grid_supported && evidence.observed_beats>100 && !beats.is_empty());
        assert!(evidence.warning().is_none());
        let a=Analysis{rhythm:Some(evidence),..Default::default()};
        let mut json=serde_json::to_value(a).unwrap();
        let restored:Analysis=serde_json::from_value(json.clone()).unwrap();assert!(restored.rhythm.unwrap().grid_supported);
        json.as_object_mut().unwrap().remove("rhythm");
        let legacy:Analysis=serde_json::from_value(json).unwrap();assert!(legacy.rhythm.is_none());
    }
    fn noise(sr:u32,seconds:usize,amplitude:f32)->Vec<f32> {
        let mut state=0x7155_12abu32;
        (0..sr as usize*seconds).map(|_|{state=state.wrapping_mul(1664525).wrapping_add(1013904223);((state>>8) as f32/8388608.0-1.0)*amplitude}).collect()
    }
    #[test]
    fn one_kick_with_quiet_noise_does_not_establish_a_beat_grid() {
        let sr=22050;let (mut signal,_)=crate::testgen::synth_kicks(&[0.37],60.0,sr);
        for (x,n) in signal.iter_mut().zip(noise(sr,60,0.0001)) {*x+=n;}
        let (_,_,beats)=detect_beats(&signal,sr);
        assert!(beats.is_empty(),"one attack and noise generated {} trusted grid markers",beats.len());
    }
    #[test]
    fn stationary_noise_does_not_establish_a_beat_grid() {
        for seconds in [8,16,60] {
            let (_,_,beats)=detect_beats(&noise(22050,seconds,0.05),22050);
            assert!(beats.is_empty(),"{seconds}s stationary noise generated {} trusted grid markers",beats.len());
        }
    }
    #[test]
    fn sustained_tone_does_not_establish_a_beat_grid() {
        for hz in [55.0,440.0] {
            let sr=22050;let m:Vec<_>=(0..sr*60).map(|i|0.2*(std::f64::consts::TAU*hz*i as f64/sr as f64).sin() as f32).collect();
            let (_,_,beats)=detect_beats(&m,sr);assert!(beats.is_empty(),"steady {hz}Hz tone generated {} trusted markers",beats.len());
        }
    }
    #[test]
    fn quiet_periodic_recording_retains_precision() {
        let sr=22050;let times:Vec<_>=(0..128).map(|k|0.37+k as f64*60.0/123.4).collect();
        let (mut m,_)=crate::testgen::synth_kicks(&times,65.0,sr);
        for x in &mut m {*x*=0.000001;}
        let (bpm,_,beats)=detect_beats(&m,sr);
        assert!((bpm-123.4).abs()<0.01,"quiet rhythm BPM changed: {bpm}");
        assert!(times[4..120].iter().all(|t|beats.iter().any(|b|(b-t).abs()<0.005)),"quiet rhythm lost supported grid");
    }

    #[test]
    #[ignore = "requires local source recordings; reports estimates without claiming ground truth"]
    fn inspect_saved_sources_without_changing_the_project() {
        let path=std::env::var("MIXSTUDIO_DIAGNOSTIC_PROJECT").unwrap();
        let bytes=std::fs::read(&path).unwrap();
        let mix:crate::model::Mix=serde_json::from_slice(&bytes).unwrap();
        for (i,t) in mix.tracks.iter().enumerate() {
            let (audio,title,artist)=decode(Path::new(&t.path)).unwrap();
            let an=analyse(&audio,title,artist,Path::new(&t.path));
            eprintln!("source {} sr={} old_bpm={:.7} new_bpm={:.7} beats={} duration={:.2}s",i+1,audio.sr,t.bpm,an.bpm,an.beats.len(),an.duration);
            assert!(an.bpm.is_finite() && an.beats.windows(2).all(|w| w[1]>w[0]));
        }
        assert_eq!(std::fs::read(path).unwrap(),bytes);
    }
}

pub struct Audio { pub sr: u32, pub l: Vec<f32>, pub r: Vec<f32> }
impl Audio { pub fn secs(&self) -> f64 { self.l.len() as f64 / self.sr as f64 } }

/// Samples per waveform peak column (at the track's own sample rate).
/// Bump when BPM/downbeat/key analysis changes so cached results are recomputed.
pub const ANALYSIS_VERSION: u32 = 11;

pub const PEAK_HOP: usize = 441;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Analysis {
    pub bpm: f64,
    pub offset: f64, // seconds to first downbeat
    pub key: String, // Camelot
    pub duration: f64,
    pub peaks: Vec<[u8; 3]>, // low, mid, high (0..255), one per PEAK_HOP samples
    pub peak_sr: u32,
    pub sections: Vec<(f64, u8)>, // phrases: (track beat, crate::phrase::LABELS index)
    pub title: String,
    pub artist: String,
    #[serde(default)] pub version: u32,
    /// beat map: time (s) of every beat
    #[serde(default)] pub beats: Vec<f64>,
    /// flex beat map (piecewise, follows deliberate tempo jumps and ramps)
    #[serde(default)] pub flex: Vec<f64>,
    #[serde(default)] pub rhythm: Option<RhythmEvidence>,
    /// Legacy cache hint. Keep Dynamic as the safe default; Flex requires manual selection.
    #[serde(default = "dynamic_grid")] pub recommended_grid_mode: u8,
}
#[derive(Clone,Serialize,Deserialize,Default)]
pub struct RhythmEvidence {
    /// Correlation, not a probability or musical downbeat confidence.
    pub periodicity:f64,
    pub observed_beats:usize,
    pub predicted_beats:usize,
    pub supported_fraction:[f64;3],
    /// Minimum evidence gates passed; not a guarantee of beat/downbeat accuracy.
    pub grid_supported:bool,
}
impl RhythmEvidence {
    pub fn warning(&self)->Option<&'static str> {
        if !self.grid_supported {Some("Source analysis: insufficient rhythm evidence. BPM is provisional; set and check the grid manually.")}
        else if self.supported_fraction.iter().any(|s|*s<0.35) {Some("Source analysis: weak attack support in parts of this song. Check the grid through the intro, middle and outro.")}
        else {None}
    }
}
fn dynamic_grid() -> u8 { 1 }

impl Analysis {
    pub fn flex_map(&self) -> Option<std::sync::Arc<crate::beatmap::BeatMap>> {
        if self.flex.len() < 16 { None } else { Some(std::sync::Arc::new(crate::beatmap::BeatMap { beats: self.flex.clone(), bpm: self.bpm })) }
    }
    pub fn beat_map(&self) -> Option<std::sync::Arc<crate::beatmap::BeatMap>> {
        if self.beats.len() < 16 { None } else { Some(std::sync::Arc::new(crate::beatmap::BeatMap { beats: self.beats.clone(), bpm: self.bpm })) }
    }
}

pub fn decode(path: &Path) -> anyhow::Result<(Audio, String, String)> {
    use std::io::{Read, Seek};
    let mut f = std::fs::File::open(path)?;
    let mut header = [0u8; 12]; f.read_exact(&mut header)?; f.rewind()?;
    let wave = &header[..4] == b"RIFF" && &header[8..] == b"WAVE";
    let mss = MediaSourceStream::new(Box::new(f), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(e); }
    let mut probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;
    let (mut title, mut artist) = (String::new(), String::new());
    let mut read_meta = |m: &symphonia::core::meta::MetadataRevision| {
        for t in m.tags() {
            match t.std_key { Some(symphonia::core::meta::StandardTagKey::TrackTitle) => title = t.value.to_string(),
                Some(symphonia::core::meta::StandardTagKey::Artist) => artist = t.value.to_string(), _ => {} }
        }
    };
    if let Some(m) = probed.metadata.get() { if let Some(r) = m.current() { read_meta(r); } }
    let mut fmt = probed.format;
    if let Some(r) = fmt.metadata().current() { read_meta(r); }
    let track = fmt.default_track().ok_or_else(|| anyhow::anyhow!("no audio track"))?.clone();
    let codecs = symphonia::default::get_codecs();
    let mut dec = codecs.make(&track.codec_params, &DecoderOptions { verify: true })?;
    // WAVE PCM and native FLAC declare samples. Other container durations may
    // include delay/padding or be estimates. Symphonia 0.5's AIFF count includes
    // the SSND header bytes, so it cannot serve as an exact sample count either.
    let flac = track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_FLAC;
    let exact_frames = if flac || (wave && codecs.get_codec(track.codec_params.codec).is_some_and(|c| c.short_name.starts_with("pcm_"))) {
        track.codec_params.n_frames.filter(|&n| n > 0)
    } else { None };
    let mut sr = track.codec_params.sample_rate.unwrap_or(44100);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    loop {
        let pkt = match fmt.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow::anyhow!("audio packet error: {e}")),
        };
        if pkt.track_id() != track.id { continue; }
        let buf = dec.decode(&pkt).map_err(|e| anyhow::anyhow!("audio decode error: {e}"))?;
        let spec = *buf.spec();
        sr = spec.rate;
        let ch = spec.channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        for fr in sb.samples().chunks(ch) {
            l.push(fr[0]);
            r.push(if ch > 1 { fr[1] } else { fr[0] });
        }
    }
    if l.is_empty() { anyhow::bail!("could not decode audio"); }
    if let Some(expected) = exact_frames {
        let actual = l.len() as u64;
        // flacenc pads its last fixed block with zeros and includes those zeros
        // in its checksum. Verify the complete decoded stream before trimming.
        let padded_flac = flac && actual > expected
            && actual - expected < dec.codec_params().max_frames_per_packet.unwrap_or(0)
            && l[expected as usize..].iter().chain(&r[expected as usize..]).all(|&s| s == 0.0);
        if actual != expected && !padded_flac { anyhow::bail!("incomplete audio: expected {expected} frames, decoded {actual}"); }
    }
    if dec.finalize().verify_ok == Some(false) { anyhow::bail!("audio integrity check failed"); }
    if flac { if let Some(expected) = exact_frames { l.truncate(expected as usize); r.truncate(expected as usize); } }
    Ok((Audio { sr, l, r }, title, artist))
}

fn mono(a: &Audio) -> Vec<f32> { a.l.iter().zip(&a.r).map(|(x, y)| 0.5 * (x + y)).collect() }

/// Keep a single, stable stereo projection for rhythm analysis. A side-dominant
/// recording must not lose its attacks through L+R cancellation. Key analysis
/// continues to use the original mid signal.
fn rhythm_mono(a: &Audio) -> Vec<f32> {
    let mid=mono(a);
    let side:Vec<_>=a.l.iter().zip(&a.r).map(|(&l,&r)|0.5*(l-r)).collect();
    // Broadband power would select a loud sustained pad over a quieter kick.
    // Compare changes in energy instead, excluding file-edge transients, and
    // prefer the conventional mid projection when rhythmic evidence is close.
    let strength=|m:&[f32]| {
        let (onset,_)=onset_env(m,a.sr);
        onset.get(8..onset.len().saturating_sub(8)).unwrap_or(&[]).iter().map(|&v|v as f64).sum::<f64>()
    };
    if strength(&side)>strength(&mid)*1.5+1e-6 {side} else {mid}
}

/// Onset envelope: half-wave rectified difference of log energy (low-passed signal emphasised), hop 512.
fn onset_env(m: &[f32], sr: u32) -> (Vec<f32>, f64) {
    let hop = 512usize;
    let fps = sr as f64 / hop as f64;
    let a = (-2.0 * std::f32::consts::PI * 150.0 / sr as f32).exp();
    let mut lp = 0f32;
    let mut e = Vec::with_capacity(m.len() / hop + 1);
    for ch in m.chunks(hop) {
        let (mut s1, mut s2) = (0f32, 0f32);
        for &x in ch { lp = a * lp + (1.0 - a) * x; s1 += lp * lp; s2 += x * x; }
        e.push((200.0 * (2.0 * s1 + s2) / ch.len() as f32).ln_1p());
    }
    let mut o = vec![0f32; e.len()];
    for i in 1..e.len() { o[i] = (e[i] - e[i - 1]).max(0.0); }
    // remove local mean
    let w = 8usize;
    let mut out = vec![0f32; o.len()];
    for i in 0..o.len() {
        let (s, t) = (i.saturating_sub(w), (i + w + 1).min(o.len()));
        let mean = o[s..t].iter().sum::<f32>() / (t - s) as f32;
        out[i] = (o[i] - mean).max(0.0);
    }
    // light gaussian smoothing so the comb search tolerates sub-frame phase
    let k = [0.25f32, 0.5, 1.0, 0.5, 0.25];
    let sm: Vec<f32> = (0..out.len()).map(|i| (0..5).map(|j| { let t = i as i64 + j as i64 - 2; if t < 0 || t as usize >= out.len() { 0.0 } else { out[t as usize] * k[j] } }).sum()).collect();
    (sm, fps)
}

/// Comb score of the onset envelope for a given period (frames) and phase (frames).
fn comb(o: &[f32], period: f64, phase: f64) -> f32 {
    let mut s = 0f32;
    let mut t = phase;
    let mut n = 0;
    while (t as usize) + 1 < o.len() {
        let i = t as usize; let f = (t - i as f64) as f32;
        s += o[i] * (1.0 - f) + o[i + 1] * f; n += 1;
        t += period;
    }
    if n == 0 { 0.0 } else { s / n as f32 }
}

/// Choose which of the 4 beat phases is the downbeat (bar 1).
/// Scores each phase by: low-band (kick/bass) energy on that beat, onset (kick attack) strength,
/// and energy jumps at 8-bar phrase boundaries aligned to that phase.
pub fn downbeat(m: &[f32], sr: u32, bpm: f64, first_beat: f64, onset: &[f32], fps: f64) -> f64 {
    let beat = 60.0 / bpm;
    let a = (-2.0 * std::f32::consts::PI * 120.0 / sr as f32).exp();
    let mut lp = 0f32;
    let win = (0.08 * sr as f64) as usize;
    let nbeats = ((m.len() as f64 / sr as f64 - first_beat) / beat) as usize;
    if nbeats < 16 { return first_beat; }
    // low-band energy in a short window after every beat
    let mut le = vec![0f32; nbeats];
    let mut idx = 0usize;
    for b in 0..nbeats {
        let st = ((first_beat + b as f64 * beat) * sr as f64) as usize;
        while idx < st && idx < m.len() { lp = a * lp + (1.0 - a) * m[idx]; idx += 1; }
        let mut e = 0f32;
        for i in st..(st + win).min(m.len()) { lp = a * lp + (1.0 - a) * m[i]; e += lp * lp; }
        idx = (st + win).min(m.len());
        le[b] = (1e-6 + e / win as f32).ln();
    }
    let on_at = |b: usize| -> f32 { let f = ((first_beat + b as f64 * beat) * fps) as usize; (f.saturating_sub(1)..(f + 2).min(onset.len())).map(|i| onset[i]).fold(0.0, f32::max) };
    let mean = le.iter().sum::<f32>() / nbeats as f32;
    let sd = (le.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / nbeats as f32).sqrt().max(1e-3);
    let omean = (0..nbeats).map(on_at).sum::<f32>() / nbeats as f32 + 1e-6;
    let mut best = (f32::MIN, 0usize);
    for k in 0..4 {
        let (mut el, mut eo, mut n) = (0f32, 0f32, 0f32);
        let mut b = k; while b < nbeats { el += (le[b] - mean) / sd; eo += on_at(b) / omean - 1.0; n += 1.0; b += 4; }
        // phrase boundaries: |energy(4 beats after) - energy(4 beats before)| every 32 beats, best of 8 sub-phases
        let mut phrase = 0f32;
        for mph in 0..8 { let mut s = 0f32; let mut c = 0f32; let mut b = k + 4 * mph + 32;
            while b + 4 < nbeats { let pre: f32 = le[b - 4..b].iter().sum(); let post: f32 = le[b..b + 4].iter().sum(); s += (post - pre).abs() / (4.0 * sd); c += 1.0; b += 32; }
            if c > 0.0 { phrase = phrase.max(s / c); } }
        let score = el / n + 0.7 * eo / n + 0.3 * phrase;
        if score > best.0 { best = (score, k); }
    }
    first_beat + best.1 as f64 * beat
}

/// Parabolic sub-frame peak position around index i.
fn parabolic(o: &[f32], i: usize) -> f64 {
    if i == 0 || i + 1 >= o.len() { return i as f64; }
    let (a, b, c) = (o[i - 1] as f64, o[i] as f64, o[i + 1] as f64);
    let d = a - 2.0 * b + c;
    if d.abs() < 1e-12 { i as f64 } else { i as f64 + (0.5 * (a - c) / d).clamp(-0.5, 0.5) }
}

/// Fit a straight beat grid to the onset peaks nearest each predicted beat (weighted least squares,
/// two passes with outlier rejection). Returns (period frames, phase frames) or None.
fn regress_grid(o: &[f32], period: f64, phase: f64) -> Option<(f64, f64)> {
    let (mut p, mut ph) = (period, phase);
    for pass in 0..3 {
        let tol = if pass == 0 { 0.2 } else { 0.12 };
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64);
        let mut k = 0f64; let mut used = 0;
        loop {
            let t = ph + k * p; if t + p * tol + 2.0 >= o.len() as f64 { break; }
            let lo = (t - p * tol).max(0.0) as usize; let hi = (t + p * tol) as usize + 1;
            let mut bi = lo; for i in lo..hi.min(o.len()) { if o[i] > o[bi] { bi = i; } }
            let w = o[bi] as f64;
            if w > 0.0 && bi > lo && bi + 1 < hi {
                let y = parabolic(o, bi);
                sw += w; sx += w * k; sy += w * y; sxx += w * k * k; sxy += w * k * y; used += 1;
            }
            k += 1.0;
        }
        if used < 16 { return None; }
        let den = sw * sxx - sx * sx; if den.abs() < 1e-9 { return None; }
        let np = (sw * sxy - sx * sy) / den; let nph = (sy - np * sx) / sw;
        if (np / period - 1.0).abs() > 0.02 { return None; }
        p = np; ph = nph;
    }
    Some((p, ph.rem_euclid(p)))
}

pub fn detect_bpm(m: &[f32], sr: u32) -> (f64, f64) {
    if sr == 0 { return (128.0, 0.0); }
    let (o, fps) = onset_env(m, sr);
    if o.len() < 64 { return (128.0, 0.0); }
    // 1) coarse tempo: autocorrelation over the whole track, 60..200 BPM, with a mild prior around 125.
    let ac_at = |lag: f64| -> f64 { let li = lag as usize; if li + 1 >= o.len() { return 0.0; } let f = (lag - li as f64) as f32; let mut ac = 0f32;
        for i in 0..o.len().saturating_sub(li + 1) { ac += o[i] * (o[i + li] * (1.0 - f) + o[i + li + 1] * f); } ac as f64 / (o.len() - li) as f64 };
    let mut best = (0f64, 128.0f64);
    let mut bpm = 60.0;
    while bpm <= 200.0 {
        let lag = fps * 60.0 / bpm;
        // tempogram-style: also sum the 2x and 4x lags (bar coherence)
        let s = ac_at(lag) + 0.5 * ac_at(2.0 * lag) + 0.25 * ac_at(4.0 * lag);
        let w = (-0.5 * ((bpm / 125.0).log2() / 0.5).powi(2)).exp();
        let score = s * (0.5 + 0.5 * w);
        if score > best.0 { best = (score, bpm); }
        bpm += 0.1;
    }
    // octave correction into the dance range 85..175
    let mut c = best.1; while c < 85.0 { c *= 2.0; } while c > 175.0 { c /= 2.0; }
    // 2) fine comb over the full track: +-1 BPM at 0.005 BPM
    let mut fine = (0f32, c, 0.0f64);
    let mut b = c - 1.0;
    while b <= c + 1.0 {
        let p = fps * 60.0 / b;
        for k in 0..32 { let ph = p * k as f64 / 32.0; let s = comb(&o, p, ph); if s > fine.0 { fine = (s, b, ph); } }
        b += 0.005;
    }
    let mut period = fps * 60.0 / fine.1; let mut ph = fine.2;
    for k in 0..64 { let p = ph - period * 0.25 + period * 0.5 * k as f64 / 64.0; let s = comb(&o, period, p.max(0.0)); if s > comb(&o, period, ph) { ph = p.max(0.0); } }
    // 3) refine by regression of the actual beat onsets across the whole track
    if let Some((p, q)) = regress_grid(&o, period, ph) { period = p; ph = q; }
    let bpm = fps * 60.0 / period;
    // onset frame i covers samples [i*hop, (i+1)*hop); with the symmetric smoothing its peak sits at the attack (~2 ms on synthetic kicks)
    let off_beat = (ph / fps).rem_euclid(60.0 / bpm);
    let off = downbeat(m, sr, bpm, off_beat, &o, fps);
    (bpm, off)
}

const MAJ: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MIN: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

/// pitch class (0=C) + minor -> Camelot
pub fn camelot(pc: usize, minor: bool) -> String {
    let maj = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
    let n = if minor { maj[(pc + 3) % 12] } else { maj[pc] };
    format!("{}{}", n, if minor { "A" } else { "B" })
}

pub fn detect_key(m: &[f32], sr: u32) -> String {
    // downsample by 4 (box average) then FFT chromagram
    let d = 4usize;
    let x: Vec<f32> = m.chunks(d).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let fs = sr as f32 / d as f32;
    let n = 8192usize;
    if x.len() < n { return "?".into(); }
    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect();
    let mut chroma = [0f32; 12];
    let mut buf = vec![rustfft::num_complex::Complex32::new(0.0, 0.0); n];
    let mut pos = 0;
    let bin_pc: Vec<Option<usize>> = (0..n / 2).map(|k| {
        let f = k as f32 * fs / n as f32;
        if !(55.0..2000.0).contains(&f) { return None; }
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        Some((midi.round() as i32).rem_euclid(12) as usize)
    }).collect();
    while pos + n <= x.len() {
        for i in 0..n { buf[i] = rustfft::num_complex::Complex32::new(x[pos + i] * win[i], 0.0); }
        fft.process(&mut buf);
        let mut fr = [0f32; 12];
        for k in 0..n / 2 { if let Some(pc) = bin_pc[k] { fr[pc] += buf[k].norm().sqrt(); } }
        let s: f32 = fr.iter().sum::<f32>() + 1e-9;
        for i in 0..12 { chroma[i] += fr[i] / s; }
        pos += n;
    }
    let corr = |p: &[f32; 12], root: usize| -> f32 {
        let mc = chroma.iter().sum::<f32>() / 12.0; let mp = p.iter().sum::<f32>() / 12.0;
        let (mut a, mut b, mut c) = (0f32, 0f32, 0f32);
        for i in 0..12 { let x = chroma[(i + root) % 12] - mc; let y = p[i] - mp; a += x * y; b += x * x; c += y * y; }
        a / (b * c).sqrt().max(1e-9)
    };
    let mut best = (-2f32, 0usize, false);
    for r in 0..12 { for (p, mi) in [(&MAJ, false), (&MIN, true)] { let c = corr(p, r); if c > best.0 { best = (c, r, mi); } } }
    camelot(best.1, best.2)
}

pub fn peaks(a: &Audio) -> Vec<[u8; 3]> {
    let sr = a.sr as f32;
    let al = (-2.0 * std::f32::consts::PI * 200.0 / sr).exp();
    let ah = (-2.0 * std::f32::consts::PI * 2500.0 / sr).exp();
    let (mut lp, mut lp2) = (0f32, 0f32);
    let mut out = Vec::with_capacity(a.l.len() / PEAK_HOP + 1);
    let (mut pl, mut pm, mut ph) = (0f32, 0f32, 0f32);
    for (i, (x, y)) in a.l.iter().zip(&a.r).enumerate() {
        let s = 0.5 * (x + y);
        lp = al * lp + (1.0 - al) * s;
        lp2 = ah * lp2 + (1.0 - ah) * s;
        let low = lp; let high = s - lp2; let mid = lp2 - lp;
        pl = pl.max(low.abs()); pm = pm.max(mid.abs()); ph = ph.max(high.abs());
        if i % PEAK_HOP == PEAK_HOP - 1 {
            let q = |v: f32| (v.min(1.0).sqrt() * 255.0) as u8;
            out.push([q(pl), q(pm * 1.3), q(ph * 2.0)]);
            pl = 0.0; pm = 0.0; ph = 0.0;
        }
    }
    out
}

#[allow(dead_code)]
/// Very rough sections per 16 bars from low-band energy.
fn sections(p: &[[u8; 3]], peak_sr: u32, bpm: f64, off: f64) -> Vec<(f64, u8)> {
    let cols_per_beat = 60.0 / bpm * peak_sr as f64 / PEAK_HOP as f64;
    let seg = 32.0; // beats
    let mut res = Vec::new();
    let total_beats = p.len() as f64 / cols_per_beat - off * bpm / 60.0;
    let mut e = Vec::new();
    let mut b = 0.0;
    while b < total_beats {
        let s = ((b + off * bpm / 60.0) * cols_per_beat) as usize; let t = (((b + seg) + off * bpm / 60.0) * cols_per_beat) as usize;
        let t = t.min(p.len()); if s >= t { break; }
        e.push((b, p[s..t].iter().map(|v| v[0] as f32).sum::<f32>() / (t - s) as f32));
        b += seg;
    }
    let mx = e.iter().map(|x| x.1).fold(1f32, f32::max);
    for (i, &(b, v)) in e.iter().enumerate() {
        let next = e.get(i + 1).map(|x| x.1).unwrap_or(v);
        let k = if v > 0.75 * mx { 1 } else if next > v * 1.25 { 2 } else { 0 };
        res.push((b, k));
    }
    res
}

pub fn cache_key(path: &Path) -> Option<String> { cache_path(path).and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string())) }
fn cache_path(path: &Path) -> Option<std::path::PathBuf> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()); let path = canon.as_path();
    let md = std::fs::metadata(path).ok()?;
    let mut h: u64 = 1469598103934665603;
    for b in path.to_string_lossy().bytes().chain(md.len().to_le_bytes()) { h = (h ^ b as u64).wrapping_mul(1099511628211); }
    if let Ok(m) = md.modified() { if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) { h ^= d.as_secs(); } }
    let home = std::env::var("HOME").ok()?;
    Some(Path::new(&home).join(".cache/mixstudio").join(format!("{h:016x}.json")))
}

/// Analysis only (no audio kept): from the cache when present, otherwise decode + analyse + cache.
pub fn load_analysis(path: &Path) -> anyhow::Result<Analysis> {
    if let Some(c) = cache_path(path) { if let Ok(s) = std::fs::read(&c) { if let Ok(a) = serde_json::from_slice::<Analysis>(&s) { if a.version == ANALYSIS_VERSION { return Ok(a); } } } }
    Ok(load(path)?.1)
}

/// Decode + analyse, using the on-disk analysis/waveform cache when present.
pub fn load(path: &Path) -> anyhow::Result<(Audio, Analysis)> {
    let (audio, title, artist) = decode(path)?;
    let cp = cache_path(path);
    if let Some(c) = &cp { if let Ok(s) = std::fs::read(c) { if let Ok(a) = serde_json::from_slice::<Analysis>(&s) { if a.version == ANALYSIS_VERSION { return Ok((audio, a)); } } } }
    let a = analyse(&audio, title, artist, path);
    if let Some(c) = cp { let _ = std::fs::create_dir_all(c.parent().unwrap()); let _ = std::fs::write(c, serde_json::to_vec(&a).unwrap_or_default()); }
    Ok((audio, a))
}

pub fn analyse(audio: &Audio, title: String, artist: String, path: &Path) -> Analysis {
    let m = mono(audio);
    let rhythm = rhythm_mono(audio);
    let (bpm, offset, beats, evidence) = detect_rhythm(&rhythm, audio.sr);
    let key = detect_key(&m, audio.sr);
    let flex = flex_beats(&rhythm, audio.sr, bpm).0;
    // Segment boundaries alone are not evidence of a reliable beat map: real tracks
    // can produce large Flex timing jumps even when their Dynamic grid is stable.
    let recommended_grid_mode = 1;
    let peaks = peaks(audio);
    // phrases (track beat from the first downbeat, label) on the 8/16/32-bar grid
    let sections = crate::phrase::detect(&crate::phrase::bar_features(&peaks, audio.sr, PEAK_HOP, bpm, offset));
    let title = if title.is_empty() { path.file_stem().map(|s| s.to_string_lossy().replace("_", " ")).unwrap_or_default() } else { title };
    Analysis { rhythm:Some(evidence), version: ANALYSIS_VERSION, beats, bpm, offset, key, duration: audio.secs(), peaks, peak_sr: audio.sr, sections, title, artist, flex, recommended_grid_mode }
}

/// Dynamic-programming beat tracker (Ellis 2007) on the onset envelope, followed by sub-frame
/// refinement of every beat and a local weighted regression (+-8 beats) so the map follows slow
/// tempo drift smoothly. Returns beat times in seconds covering the whole track.
fn track_beats(o: &[f32], fps: f64, period: f64, phase: f64) -> Vec<f64> {
    let n = o.len(); let p = period;
    if n < (4.0 * p) as usize { return vec![]; }
    let sd = { let m = o.iter().sum::<f32>() / n as f32; (o.iter().map(|x| (x - m).powi(2)).sum::<f32>() / n as f32).sqrt().max(1e-20) };
    let on: Vec<f64> = o.iter().map(|&x| (x / sd) as f64).collect();
    let alpha = 300.0;
    let (lo, hi) = ((p * 0.85).round() as usize, (p * 1.15).round() as usize + 1);
    let mut c = on.clone(); let mut back = vec![usize::MAX; n];
    for t in lo..n {
        let mut best = (f64::MIN, usize::MAX);
        for tau in t.saturating_sub(hi)..=t - lo { let d = (t - tau) as f64 / p; let s = c[tau] - alpha * d.ln().powi(2); if s > best.0 { best = (s, tau); } }
        if best.1 != usize::MAX && best.0 > 0.0 { c[t] = on[t] + best.0; back[t] = best.1; }
    }
    // start from the best score in the last beat period
    let mut t = (n.saturating_sub(hi)..n).max_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap()).unwrap();
    let mut fr = vec![t];
    while back[t] != usize::MAX { t = back[t]; fr.push(t); }
    fr.reverse();
    // extend with the global grid where the DP chain does not reach (silent intro/outro)
    let first = fr[0] as f64; let last = *fr.last().unwrap() as f64;
    let _ = phase;
    let mut fb: Vec<f64> = vec![];
    let mut x = first - p; while x >= -0.5 * p { fb.push(x); x -= p; } fb.reverse();
    let mut w: Vec<f64> = vec![0.0; fb.len()];
    for &f in &fr {
        // sub-frame peak near the DP beat
        let mut bi = f; for i in f.saturating_sub(2)..(f + 3).min(n) { if o[i] > o[bi] { bi = i; } }
        let y = if bi > 0 && bi + 1 < n { let (a, b, cc) = (o[bi - 1] as f64, o[bi] as f64, o[bi + 1] as f64); let d = a - 2.0 * b + cc;
            if d.abs() > 1e-12 { bi as f64 + (0.5 * (a - cc) / d).clamp(-0.5, 0.5) } else { bi as f64 } } else { bi as f64 };
        let strong = on[bi] > 0.5;
        fb.push(if strong { y } else { f as f64 }); w.push(if strong { on[bi] } else { 0.02 });
    }
    let mut x = last + p; while x < n as f64 + 0.5 * p { fb.push(x); w.push(0.0); x += p; }
    // local weighted linear regression of beat time vs index (+-8 beats, triangular window)
    let m = fb.len(); let h = 8i64;
    let mut sm = vec![0f64; m];
    for k in 0..m {
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (1e-9, 0.0, 0.0, 0.0, 0.0);
        for j in (k as i64 - h).max(0)..=(k as i64 + h).min(m as i64 - 1) {
            let ww = (w[j as usize] + 1e-4) * (1.0 - (j - k as i64).abs() as f64 / (h + 1) as f64);
            let xx = (j - k as i64) as f64; sw += ww; sx += ww * xx; sy += ww * fb[j as usize]; sxx += ww * xx * xx; sxy += ww * xx * fb[j as usize];
        }
        let den = sw * sxx - sx * sx;
        sm[k] = if den.abs() > 1e-12 { let b = (sw * sxy - sx * sy) / den; (sy - b * sx) / sw } else { fb[k] };
    }
    // monotonic, spacing within +-30 % of the global period
    for k in 1..m { if sm[k] < sm[k - 1] + 0.7 * p { sm[k] = sm[k - 1] + 0.7 * p; } }
    sm.iter().map(|f| f / fps).collect()
}

/// Choose which of the 4 beat phases is bar 1 on a beat list (low-band energy, kick attack, 8-bar phrase jumps).
fn downbeat_idx(m: &[f32], sr: u32, beats: &[f64], onset: &[f32], fps: f64) -> usize {
    let nb = beats.len(); if nb < 16 { return 0; }
    let a = (-2.0 * std::f32::consts::PI * 120.0 / sr as f32).exp();
    let win = (0.08 * sr as f64) as usize;
    let mut le = vec![0f32; nb];
    for (b, &t) in beats.iter().enumerate() {
        let st = ((t.max(0.0)) * sr as f64) as usize; let mut lp = 0f32; let mut e = 0f32;
        for i in st.saturating_sub(256)..st { if i < m.len() { lp = a * lp + (1.0 - a) * m[i]; } }
        for i in st..(st + win).min(m.len()) { lp = a * lp + (1.0 - a) * m[i]; e += lp * lp; }
        le[b] = (1e-6 + e / win as f32).ln();
    }
    let on_at = |b: usize| -> f32 { let f = (beats[b] * fps).max(0.0) as usize; (f.saturating_sub(1)..(f + 2).min(onset.len())).map(|i| onset[i]).fold(0.0, f32::max) };
    let mean = le.iter().sum::<f32>() / nb as f32;
    let sd = (le.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / nb as f32).sqrt().max(1e-3);
    let omean = (0..nb).map(on_at).sum::<f32>() / nb as f32 + 1e-6;
    let mut best = (f32::MIN, 0usize);
    for k in 0..4 {
        let (mut el, mut eo, mut n) = (0f32, 0f32, 0f32);
        let mut b = k; while b < nb { el += (le[b] - mean) / sd; eo += on_at(b) / omean - 1.0; n += 1.0; b += 4; }
        let mut phrase = 0f32;
        for mph in 0..8 { let mut s = 0f32; let mut c = 0f32; let mut b = k + 4 * mph + 32;
            while b + 4 < nb { let pre: f32 = le[b - 4..b].iter().sum(); let post: f32 = le[b..b + 4].iter().sum(); s += (post - pre).abs() / (4.0 * sd); c += 1.0; b += 32; }
            if c > 0.0 { phrase = phrase.max(s / c); } }
        let score = el / n + 0.7 * eo / n + 0.3 * phrase;
        if score > best.0 { best = (score, k); }
    }
    best.1
}

/// Normalised onset covariance at the tempo family. A random noise floor must
/// not become beat evidence just because DP can select local maxima from it.
fn tempo_support(onset:&[f32],fps:f64,bpm:f64)->f64 {
    if onset.len()<64 || !bpm.is_finite() || bpm<=0.0 {return 0.0;}
    let mean=onset.iter().map(|v|*v as f64).sum::<f64>()/onset.len() as f64;
    [1.0,2.0,4.0].iter().map(|multiple|{
        let lag=fps*60.0/bpm*multiple;let k=lag as usize;let f=lag-k as f64;
        // A handful of overlapping pairs can correlate by chance, especially
        // at the fourth lag in a short clip. Require several repeated cycles.
        if onset.len().saturating_sub(k+1)<k.max(16)*4 {return 0.0;}
        let mut xx=0.0;let mut yy=0.0;let mut xy=0.0;
        for i in 0..onset.len().saturating_sub(k+1) {
            let x=onset[i] as f64-mean;let y=onset[i+k] as f64*(1.0-f)+onset[i+k+1] as f64*f-mean;
            xx+=x*x;yy+=y*y;xy+=x*y;
        }
        if xx*yy>1e-40 {xy/(xx*yy).sqrt()}else{0.0}
    }).fold(0.0,f64::max)
}

/// Full beat analysis: global BPM, estimated first downbeat and beat-map seconds.
pub fn detect_beats(m: &[f32], sr: u32) -> (f64, f64, Vec<f64>) {
    let (bpm,offset,beats,_)=detect_rhythm(m,sr);(bpm,offset,beats)
}
fn detect_rhythm(m:&[f32],sr:u32)->(f64,f64,Vec<f64>,RhythmEvidence) {
    let mut evidence=RhythmEvidence::default();
    if sr == 0 || m.is_empty() || !m.iter().any(|x| x.abs() > 1e-8) { return (128.0, 0.0, vec![],evidence); }
    let (bpm, off_lin) = detect_bpm(m, sr);
    let (o, fps) = onset_env(m, sr);
    let support=tempo_support(&o,fps,bpm);evidence.periodicity=support;
    if support<0.12 {return (bpm,off_lin,vec![],evidence);}
    let mut beats = track_beats(&o, fps, fps * 60.0 / bpm, off_lin * fps);
    while beats.first().is_some_and(|&b| b < -0.05) { beats.remove(0); }
    let (refined,observed)=refine_attacks(m, sr, &beats);
    beats=refined;
    evidence.observed_beats=observed.len();evidence.predicted_beats=beats.len();
    let duration=m.len() as f64/sr as f64;
    for third in 0..3 {
        let lo=duration*third as f64/3.0;let hi=duration*(third+1) as f64/3.0;
        let predicted=beats.iter().filter(|b|**b>=lo && **b<hi).count();
        let supported=observed.iter().filter(|b|b.1>=lo && b.1<hi).count();
        evidence.supported_fraction[third]=if predicted>0 {(supported as f64/predicted as f64).min(1.0)}else{0.0};
    }
    if observed.len()<16 {return (bpm,off_lin,vec![],evidence); }
    while beats.first().is_some_and(|&b| b < -0.02) { beats.remove(0); }
    if let Some(b) = beats.first_mut() { *b = b.max(0.0); }
    if beats.len() < 16 { return (bpm, off_lin, vec![],evidence); }
    // The coarse energy envelope locates the tempo family. For steady material,
    // fit the final sample-level attacks instead: frame quantisation and phase
    // aliasing in a long-track comb must not become a wrong fixed BPM. Retain
    // the original estimate for variable/uncertain grids.
    let bpm = steady_tempo(&observed, bpm).unwrap_or(bpm);
    // bar phase from the global grid (robust), snapped to the nearest tracked beat; offset = first such downbeat
    let near = beats.iter().enumerate().min_by(|x, y| (x.1 - off_lin).abs().partial_cmp(&(y.1 - off_lin).abs()).unwrap()).map(|x| x.0).unwrap_or(0);
    let _ = downbeat_idx;
    let k = near % 4;
    evidence.grid_supported=true;
    (bpm, beats[k], beats,evidence)
}

fn steady_tempo(beats: &[(f64,f64)], coarse_bpm: f64) -> Option<f64> {
    if beats.len() < 32 { return None; }
    let points = &beats[2..beats.len()-2];
    let span=points.last()?.0-points.first()?.0;
    if span<32.0 || points.len() as f64/(span+1.0)<0.6 {return None;}
    let centre = points.iter().map(|p|p.0).sum::<f64>()/points.len() as f64;
    let mean = points.iter().map(|p|p.1).sum::<f64>() / points.len() as f64;
    let (mut xx, mut xy) = (0.0, 0.0);
    for &(beat,t) in points { let x = beat-centre; xx += x*x; xy += x*(t-mean); }
    let period = xy / xx;
    let bpm = 60.0 / period;
    if !bpm.is_finite() || (bpm/coarse_bpm-1.0).abs() > 0.02 { return None; }
    let mut residuals: Vec<_> = points.iter().map(|&(beat,t)| (t-mean-(beat-centre)*period).abs()).collect();
    residuals.sort_by(f64::total_cmp);
    if residuals[residuals.len()*9/10] > 0.010 { return None; }
    Some(bpm)
}

/// Sample-accurate refinement: move every beat to the steepest rise of the low-band (kick) envelope
/// within +-25 ms, then re-smooth with a +-8 beat weighted local regression. Beats without a clear
/// attack keep their position with a small weight.
fn refine_attacks(m: &[f32], sr: u32, beats: &[f64]) -> (Vec<f64>,Vec<(f64,f64)>) {
    let n = beats.len(); if n < 16 { return (beats.to_vec(),vec![]); }
    let a = (-2.0 * std::f32::consts::PI * 150.0 / sr as f32).exp(); let ae = (-1.0 / (0.002 * sr as f32)).exp();
    let (mut y1, mut y2, mut ev) = (0f32, 0f32, 0f32);
    // ripple-free quadrature envelope of the low band (|y| combined with its derivative scaled to ~60 Hz), so the
    // attack is located without the lag of rectify-and-smooth; matches a Hilbert envelope to ~1 ms on kicks
    let wq = sr as f32 / (2.0 * std::f32::consts::PI * 60.0); let _ = ae; let mut prev = 0f32;
    let env: Vec<f32> = m.iter().map(|&x| { y1 = a * y1 + (1.0 - a) * x; y2 = a * y2 + (1.0 - a) * y1; let dy = (y2 - prev) * wq; prev = y2; ev = (y2 * y2 + dy * dy).sqrt(); ev }).collect();
    let d = (0.001 * sr as f64) as usize; let w = (0.012 * sr as f64) as usize;
    // Global attack offset: average the kick envelope around all beats (+-80 ms) and take its steepest rise.
    // The onset-envelope beats lag the real attack by a track-dependent amount (tens of ms for kicks with a
    // soft attack and long sub tail), so a fixed local search window alone cannot reach it.
    let fw = (0.080 * sr as f64) as i64; let step = 8i64;
    let mut fold = vec![0f64; (2 * fw / step) as usize + 1];
    for &t in beats { let c = (t * sr as f64) as i64; if c - fw < 0 || (c + fw) as usize + d >= env.len() { continue; }
        for (i, f) in fold.iter_mut().enumerate() { let q = (c - fw + i as i64 * step) as usize; *f += (env[q + d] - env[q]) as f64; } }
    let gi = fold.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|x| x.0).unwrap_or((fw / step) as usize);
    let g = ((gi as i64 * step - fw) as f64) / sr as f64;
    let beats: Vec<f64> = beats.iter().map(|t| t + g).collect(); let beats = &beats[..];
    let mut obs = vec![(0f64, 0f64); n];
    for (k, &t) in beats.iter().enumerate() {
        let c = (t * sr as f64) as i64; if c - (w as i64) < 0 || (c as usize) + w + d >= env.len() { obs[k] = (t, 0.0); continue; }
        let c = c as usize; let mut best = (0f32, c);
        for i in (c - w..c + w).step_by(4) { let r = env[i + d] - env[i]; if r > best.0 { best = (r, i); } }
        // A carrier/noise ripple has a steep local slope too. Require a sustained
        // increase across several cycles, independent of recording gain.
        let width=(0.008*sr as f64).round().max(1.0) as usize;
        let j=best.1;
        let contrast=if j>=width*3 && j+width*2<env.len() {
            let pre=env[j-width*3..j-width].iter().map(|x|*x as f64).sum::<f64>()/(width*2) as f64;
            let post=env[j+width..j+width*2].iter().map(|x|*x as f64).sum::<f64>()/width as f64;
            post>pre*1.20+1e-20
        } else {false};
        obs[k] = (best.1 as f64 / sr as f64, if contrast {best.0 as f64}else{0.0});
    }
    let mut rs: Vec<f64> = obs.iter().map(|o| o.1).collect(); rs.sort_by(|a, b| a.partial_cmp(b).unwrap()); let med = rs[n / 2].max(1e-9);
    let strongest=rs.get((n-1)*95/100).copied().unwrap_or(0.0);
    let threshold=(0.3*med).max(strongest*0.02);
    let reliable:Vec<usize>=(0..n).filter(|&k|obs[k].1>threshold).collect();
    let pts: Vec<(f64, f64)> = obs.iter().enumerate().map(|(k,o)| {
        if o.1>threshold { return (o.0,(o.1/med).min(3.0)); }
        // Do not let quantised DP predictions pull the grid during a breakdown.
        // Missing attacks are interpolated between actual attacks, never treated
        // as new observations. Retain weak weight at uncertain intro/outro edges.
        let next=reliable.partition_point(|&j|j<k);
        let t=if next>0 && next<reliable.len() {
            let (a,b)=(reliable[next-1],reliable[next]);
            obs[a].0+(obs[b].0-obs[a].0)*(k-a) as f64/(b-a) as f64
        } else { beats[k] };
        (t,0.01)
    }).collect();
    let h = 8i64;
    let mut out: Vec<f64> = (0..n).map(|k| {
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (1e-12, 0.0, 0.0, 0.0, 0.0);
        for j in (k as i64 - h).max(0)..=(k as i64 + h).min(n as i64 - 1) {
            let (y, wt) = pts[j as usize]; let ww = wt * (1.0 - (j - k as i64).abs() as f64 / (h + 1) as f64); let x = (j - k as i64) as f64;
            sw += ww; sx += ww * x; sy += ww * y; sxx += ww * x * x; sxy += ww * x * y; }
        let den = sw * sxx - sx * sx; if den.abs() > 1e-15 { let b = (sw * sxy - sx * sy) / den; (sy - b * sx) / sw } else { beats[k] } }).collect();
    let p = (beats[n - 1] - beats[0]) / (n - 1) as f64;
    for k in 1..n { if out[k] < out[k - 1] + 0.7 * p { out[k] = out[k - 1] + 0.7 * p; } }
    let observed=reliable.iter().map(|&k|(k as f64,obs[k].0)).collect();
    (out,observed)
}

/// Flex beat map for tracks with deliberate tempo changes (jumps such as 100 -> 128 BPM, or ramps):
/// local tempo from the onset autocorrelation every second, change-point segmentation of that tempo
/// curve into linear pieces (dynamic programming with a per-segment penalty), dynamic-programming beat
/// tracking with the piecewise local period, and attack refinement without any smoothing across
/// change points. Returns beat times (s) and the change points (s).
pub fn flex_beats(m: &[f32], sr: u32, bpm: f64) -> (Vec<f64>, Vec<f64>) {
    let (o, fps) = onset_env(m, sr);
    let n = o.len(); if n < (fps * 10.0) as usize || bpm <= 0.0 { return (vec![], vec![]); }
    // 1. local tempo every second (6 s windows), lags within bpm / 1.5 .. bpm * 1.5
    let win = (fps * 6.0) as usize; let hop = fps.round() as usize;
    let (lmin, lmax) = ((fps * 60.0 / (bpm * 1.5)).floor().max(2.0) as usize, (fps * 60.0 / (bpm / 1.5)).ceil() as usize);
    let mut pts: Vec<(f64, f64)> = vec![]; // (seconds, bpm)
    let mut c = 0usize;
    while c + win <= n {
        let w = &o[c..c + win];
        let ac = |l: usize| -> f64 { if l + 1 >= win { return 0.0; } (0..win - l).map(|i| (w[i] * w[i + l]) as f64).sum::<f64>() / (win - l) as f64 };
        let sc: Vec<f64> = (lmin..=lmax + 1).map(|l| ac(l) + 0.5 * ac(2 * l)).collect();
        let k = (1..sc.len() - 1).max_by(|&a, &b| sc[a].partial_cmp(&sc[b]).unwrap()).unwrap_or(1);
        let (a, b, cc) = (sc[k - 1], sc[k], sc[k + 1]); let d = a - 2.0 * b + cc;
        let lag = (lmin + k) as f64 + if d.abs() > 1e-12 { (0.5 * (a - cc) / d).clamp(-0.5, 0.5) } else { 0.0 };
        if b > 1e-9 { pts.push(((c + win / 2) as f64 / fps, 60.0 * fps / lag)); }
        c += hop;
    }
    if pts.len() < 3 { return (vec![], vec![]); }
    // 2. piecewise-linear segmentation (optimal partition, penalty per segment)
    let k = pts.len(); let pen = 40.0;
    let sse = |i: usize, j: usize| -> f64 { // points i..j (exclusive), least-squares line
        let nn = (j - i) as f64; if nn < 2.0 { return 0.0; }
        let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
        for p in &pts[i..j] { sx += p.0; sy += p.1; sxx += p.0 * p.0; sxy += p.0 * p.1; }
        let den = nn * sxx - sx * sx; let b = if den.abs() > 1e-12 { (nn * sxy - sx * sy) / den } else { 0.0 }; let a = (sy - b * sx) / nn;
        pts[i..j].iter().map(|p| (p.1 - a - b * p.0).powi(2)).sum() };
    let mut best = vec![f64::MAX; k + 1]; let mut from = vec![0usize; k + 1]; best[0] = 0.0;
    for j in 1..=k { for i in 0..j { let v = best[i] + sse(i, j) + pen; if v < best[j] { best[j] = v; from[j] = i; } } }
    let mut cuts = vec![k]; let mut j = k; while j > 0 { j = from[j]; cuts.push(j); } cuts.reverse();
    // drop the short transition segments (windows straddling a jump) and fit each remaining segment
    let mut segs: Vec<(f64, f64, f64, f64)> = vec![]; // (t0, t1, a, b): bpm = a + b t
    for w in cuts.windows(2) { let (i, j) = (w[0], w[1]); if j - i < 3 { continue; }
        let nn = (j - i) as f64; let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
        for p in &pts[i..j] { sx += p.0; sy += p.1; sxx += p.0 * p.0; sxy += p.0 * p.1; }
        let den = nn * sxx - sx * sx; let b = if den.abs() > 1e-12 { (nn * sxy - sx * sy) / den } else { 0.0 }; let a = (sy - b * sx) / nn;
        segs.push((pts[i].0, pts[j - 1].0, a, b)); }
    if segs.is_empty() { return (vec![], vec![]); }
    let changes: Vec<f64> = segs.windows(2).map(|w| 0.5 * (w[0].1 + w[1].0)).collect();
    let tempo_at = |t: f64| -> f64 { let s = segs.iter().position(|s| t <= s.1).map(|i| if i > 0 && t < segs[i].0 { if t - segs[i - 1].1 < segs[i].0 - t { i - 1 } else { i } } else { i }).unwrap_or(segs.len() - 1);
        let s = &segs[s]; let tt = t.clamp(s.0, s.1); (s.2 + s.3 * tt).clamp(30.0, 300.0) };
    // 3. beat tracking with the local period
    let sd = { let mu = o.iter().sum::<f32>() / n as f32; (o.iter().map(|x| (x - mu).powi(2)).sum::<f32>() / n as f32).sqrt().max(1e-6) };
    let on: Vec<f64> = o.iter().map(|&x| (x / sd) as f64).collect();
    let mut cs = on.clone(); let mut back = vec![usize::MAX; n];
    for t in 1..n { let p = 60.0 * fps / tempo_at(t as f64 / fps);
        let (lo, hi) = ((p * 0.7).round().max(1.0) as usize, (p * 1.4).round() as usize + 1); if t < lo { continue; }
        let mut bst = (f64::MIN, usize::MAX);
        for tau in t.saturating_sub(hi)..=t - lo { let d = (t - tau) as f64 / p; let s = cs[tau] - 120.0 * d.ln().powi(2); if s > bst.0 { bst = (s, tau); } }
        if bst.1 != usize::MAX && bst.0 > 0.0 { cs[t] = on[t] + bst.0; back[t] = bst.1; } }
    let pend = 60.0 * fps / tempo_at(n as f64 / fps);
    let mut t = (n.saturating_sub(pend as usize + 1)..n).max_by(|&a, &b| cs[a].partial_cmp(&cs[b]).unwrap()).unwrap();
    let mut fr = vec![t]; while back[t] != usize::MAX { t = back[t]; fr.push(t); } fr.reverse();
    let beats: Vec<f64> = fr.iter().map(|&f| { let mut bi = f; for i in f.saturating_sub(2)..(f + 3).min(n) { if o[i] > o[bi] { bi = i; } }
        let y = if bi > 0 && bi + 1 < n { let (a, b, c) = (o[bi - 1] as f64, o[bi] as f64, o[bi + 1] as f64); let d = a - 2.0 * b + c; if d.abs() > 1e-12 { bi as f64 + (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { bi as f64 } } else { bi as f64 };
        y / fps }).collect();
    (flex_attacks(m, sr, &beats), changes)
}

/// Move every beat to the steepest rise of the low-band envelope near it (one global lag, then a
/// +-12 ms local search); no smoothing, so tempo jumps stay sharp.
fn flex_attacks(m: &[f32], sr: u32, beats: &[f64]) -> Vec<f64> {
    let n = beats.len(); if n < 8 { return beats.to_vec(); }
    let a = (-2.0 * std::f32::consts::PI * 150.0 / sr as f32).exp();
    let (mut y1, mut y2, mut prev) = (0f32, 0f32, 0f32); let wq = sr as f32 / (2.0 * std::f32::consts::PI * 60.0);
    let env: Vec<f32> = m.iter().map(|&x| { y1 = a * y1 + (1.0 - a) * x; y2 = a * y2 + (1.0 - a) * y1; let dy = (y2 - prev) * wq; prev = y2; (y2 * y2 + dy * dy).sqrt() }).collect();
    let d = (0.001 * sr as f64) as usize; let w = (0.012 * sr as f64) as usize;
    let fw = (0.080 * sr as f64) as i64; let step = 8i64; let mut fold = vec![0f64; (2 * fw / step) as usize + 1];
    for &t in beats { let c = (t * sr as f64) as i64; if c - fw < 0 || (c + fw) as usize + d >= env.len() { continue; }
        for (i, f) in fold.iter_mut().enumerate() { let q = (c - fw + i as i64 * step) as usize; *f += (env[q + d] - env[q]) as f64; } }
    let gi = fold.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|x| x.0).unwrap_or((fw / step) as usize);
    let g = ((gi as i64 * step - fw) as f64) / sr as f64;
    let mut out: Vec<f64> = beats.iter().map(|&t0| { let t = t0 + g; let c = (t * sr as f64) as i64;
        if c - (w as i64) < 0 || (c as usize) + w + d >= env.len() { return t; }
        let c = c as usize; let mut best = (0f32, c); for i in c - w..c + w { let r = env[i + d] - env[i]; if r > best.0 { best = (r, i); } }
        if best.0 > 0.0 { best.1 as f64 / sr as f64 } else { t } }).collect();
    out.retain(|&t| t >= 0.0);
    for k in 1..out.len() { if out[k] <= out[k - 1] + 0.05 { out[k] = out[k - 1] + 0.05; } }
    out
}

#[cfg(test)]
mod decode_integrity_tests {
    use super::*;
    fn wav(frames: usize) -> Vec<u8> {
        let mut b=Vec::new();
        b.extend_from_slice(b"RIFF"); b.extend_from_slice(&(36+frames as u32*2).to_le_bytes()); b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes()); b.extend_from_slice(&1u16.to_le_bytes()); b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes()); b.extend_from_slice(&16000u32.to_le_bytes()); b.extend_from_slice(&2u16.to_le_bytes()); b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data"); b.extend_from_slice(&(frames as u32*2).to_le_bytes()); b.resize(44+frames*2,0); b
    }
    #[test]
    fn flac_decode_checks_complete_samples_and_integrity() {
        let dir=std::env::temp_dir().join(format!("mixstudio-flac-{}",crate::recovery::unique_id())); std::fs::create_dir(&dir).unwrap();
        let path=dir.join("audio.flac");
        let samples:Vec<_>=(0..32000).map(|i| (i as f32*0.137).sin()*0.4).collect();
        crate::export::flac(&path,&samples,&samples,44100).unwrap();
        let decoded=decode(&path).unwrap().0; assert_eq!(decoded.l.len(),samples.len());
        let mut bytes=std::fs::read(&path).unwrap(); let n=bytes.len(); bytes[n-64]^=0x20;
        std::fs::write(&path,&bytes).unwrap(); let corrupt=decode(&path);
        bytes.truncate(n*3/4); std::fs::write(&path,&bytes).unwrap(); let truncated=decode(&path);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(corrupt.is_err(),"corrupted FLAC payload accepted"); assert!(truncated.is_err(),"truncated FLAC accepted");
    }
    #[test]
    fn compressed_delay_and_padding_are_not_exact_pcm_counts() {
        // Exercise both a duration-estimating raw MP3 and Xing/LAME delay/padding.
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() { return; }
        let dir=std::env::temp_dir().join(format!("mixstudio-lossy-{}",crate::recovery::unique_id())); std::fs::create_dir(&dir).unwrap();
        let source=dir.join("source.wav"); std::fs::write(&source,wav(10001)).unwrap();
        for (name,opts) in [("xing.mp3",vec!["-c:a","libmp3lame"]),("raw.mp3",vec!["-c:a","libmp3lame","-write_xing","0"]),("audio.m4a",vec!["-c:a","aac"]),("audio.ogg",vec!["-c:a","libvorbis"]),("audio.aiff",vec!["-c:a","pcm_s16be"]),("alac.m4a",vec!["-c:a","alac"])] {
            let path=dir.join(name);
            let status=std::process::Command::new("ffmpeg").args(["-v","error","-nostdin","-i"]).arg(&source).args(opts).arg(&path).status().unwrap();
            assert!(status.success(),"fixture encoder failed: {name}");
            let result=decode(&path); assert!(result.is_ok(),"valid {name} rejected: {:?}",result.err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn truncated_wav_is_not_successful_audio() {
        let dir=std::env::temp_dir().join(format!("mixstudio-decode-{}",crate::recovery::unique_id())); std::fs::create_dir(&dir).unwrap();
        let path=dir.join("audio.wav"); let mut bytes=wav(8000); std::fs::write(&path,&bytes).unwrap();
        assert_eq!(decode(&path).unwrap().0.l.len(),8000);
        bytes.truncate(44+4000); std::fs::write(&path,&bytes).unwrap();
        let result=decode(&path); std::fs::remove_dir_all(dir).unwrap();
        assert!(result.is_err(),"a WAV missing 75% of its declared samples was accepted");
    }
}
