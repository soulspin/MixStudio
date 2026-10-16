//! Opt-in, read-only real-file diagnostics. No reference annotations are assumed.
use super::*;
use serde_json::{json, Value};
use sha2::Digest;
use std::path::PathBuf;

fn distribution(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() { return json!({"count":0}); }
    let n=values.len();
    let signed_median=values[n/2];
    let mut absolute:Vec<_>=values.iter().map(|v|v.abs()).collect();
    absolute.sort_by(f64::total_cmp);
    json!({"count":n,"signed_median_ms":signed_median*1000.0,
        "signed_p10_ms":values[(n-1)/10]*1000.0,"signed_p90_ms":values[(n-1)*9/10]*1000.0,
        "absolute_median_ms":absolute[n/2]*1000.0,"absolute_p95_ms":absolute[(n-1)*95/100]*1000.0,
        "within_10ms_fraction":absolute.iter().filter(|v|**v<=0.010).count() as f64/n as f64,
        "within_25ms_fraction":absolute.iter().filter(|v|**v<=0.025).count() as f64/n as f64})
}

fn evidence(audio:&Audio, beats:&[f64], nominal_period:f64) -> Value {
    let mut thirds=Vec::new();
    for third in 0..3 {
        let lo=audio.secs()*third as f64/3.0;
        let hi=audio.secs()*(third+1) as f64/3.0;
        let indices:Vec<_>=beats.iter().enumerate().filter(|(_,b)|**b>=lo && **b<hi).map(|(i,_)|i).collect();
        let stride=indices.len().div_ceil(120).max(1);
        let mut bass=Vec::new();let mut high=Vec::new();let mut combined=Vec::new();let mut sampled=0;let mut conflicts=0;
        for &i in indices.iter().step_by(stride) {
            let period=if i+1<beats.len() {(beats[i+1]-beats[i]).clamp(0.20,1.5)}else{nominal_period};
            let (b,h,c)=crate::beatcheck::diagnostic_attacks(audio,beats[i],period);
            sampled+=1;
            if let Some(v)=b {bass.push(v);}
            if let Some(v)=h {high.push(v);}
            if b.zip(h).is_some_and(|(x,y)|(x-y).abs()>0.015) {conflicts+=1;}
            if let Some((v,_))=c {combined.push(v);}
        }
        thirds.push(json!({"third":(["early","middle","late"][third]),"start_seconds":lo,"end_seconds":hi,
            "grid_beats_in_third":indices.len(),"sampled_beats":sampled,"conflicting_bass_percussion":conflicts,
            "combined_support_fraction":if sampled>0 {combined.len() as f64/sampled as f64}else{0.0},
            "bass_residual":distribution(bass),"percussion_residual":distribution(high),"combined_residual":distribution(combined)}));
    }
    json!(thirds)
}

fn corpus_files() -> Vec<PathBuf> {
    if let Ok(manifest)=std::env::var("MIXSTUDIO_CORPUS_MANIFEST") {
        let text=std::fs::read_to_string(manifest).expect("read manifest (one audio path per line)");
        return text.lines().filter(|s|!s.trim().is_empty() && !s.trim().starts_with('#')).map(|s|PathBuf::from(s.trim())).collect();
    }
    let root=std::env::var("MIXSTUDIO_CORPUS_DIR").map(PathBuf::from).unwrap_or_else(|_|PathBuf::from(std::env::var("HOME").unwrap()).join("Downloads"));
    let mut files=Vec::new();
    for dir in [root.clone(),root.join("nt")] {
        if let Ok(entries)=std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path=entry.path();
                if path.extension().is_some_and(|x|["mp3","wav","flac","aiff","m4a"].contains(&x.to_string_lossy().to_lowercase().as_str())) {files.push(path);}
            }
        }
    }
    files.sort();files
}

#[test]
#[ignore = "requires local audio; fresh read-only estimates and agreement proxies, no annotated accuracy"]
fn inspect_local_corpus_without_mutating_sources() {
    let files=corpus_files();assert!(!files.is_empty(),"no local corpus files");
    let output=std::env::var("MIXSTUDIO_CORPUS_OUTPUT").unwrap_or_else(|_|"/tmp/mixstudio-corpus-baseline.json".into());
    let mut results=Vec::new();
    for path in &files {
        eprintln!("corpus analysing {}",path.display());
        let before=std::fs::metadata(path).unwrap();
        let source_sha256=format!("{:x}",sha2::Sha256::digest(std::fs::read(path).unwrap()));
        let (audio,title,artist)=decode(path).expect("decode local audio");
        let an=analyse(&audio,title,artist,path);
        let rhythm=rhythm_mono(&audio);
        let period=60.0/an.bpm;
        let static_beats:Vec<_>=(0..((an.duration-an.offset).max(0.0)/period).ceil() as usize).map(|i|an.offset+i as f64*period).collect();
        let mut local=Vec::new();
        for third in 0..3 {
            let center=audio.secs()*(third as f64+0.5)/3.0;
            let start=(center-22.5).max(0.0);let end=(start+45.0).min(audio.secs());
            let lo=(start*audio.sr as f64) as usize;let hi=((end*audio.sr as f64) as usize).min(rhythm.len());
            let (bpm,offset)=detect_bpm(&rhythm[lo..hi],audio.sr);
            local.push(json!({"third":(["early","middle","late"][third]),"start_seconds":start,"end_seconds":end,
                "estimated_bpm":bpm,"relative_bpm_difference_percent":(bpm/an.bpm-1.0)*100.0,"local_offset_seconds":offset}));
        }
        let result=json!({"path":path,"file_bytes":before.len(),"source_sha256":source_sha256,"sample_rate":audio.sr,"frames":audio.l.len(),
            "duration_seconds":audio.secs(),"analysis_version":an.version,"rhythm_evidence":an.rhythm,"estimated_bpm":an.bpm,"offset_seconds":an.offset,
            "dynamic_map_beat_count":an.beats.len(),"flex_map_beat_count":an.flex.len(),
            "static_grid_evidence":evidence(&audio,&static_beats,period),"dynamic_grid_evidence":evidence(&audio,&an.beats,period),
            "flex_grid_evidence":evidence(&audio,&an.flex,period),"local_window_tempo_estimates":local});
        eprintln!("corpus result {} BPM={:.7} offset={:.5}s dynamic={} flex={}",path.display(),an.bpm,an.offset,an.beats.len(),an.flex.len());
        let after=std::fs::metadata(path).unwrap();assert_eq!(before.len(),after.len());assert_eq!(before.modified().unwrap(),after.modified().unwrap());
        results.push(result);
        let unique_byte_streams=results.iter().filter_map(|v|v["source_sha256"].as_str()).collect::<std::collections::HashSet<_>>().len();
        let report=json!({"distinct_byte_streams_completed":unique_byte_streams,"analysis_version":ANALYSIS_VERSION,"file_count":files.len(),"completed_file_count":results.len(),
            "interpretation":"No annotated beat/tempo ground truth. Residuals measure agreement of candidate grids with conservative independent beatcheck attack evidence within a bounded search radius. Support is selective, and an offbeat grid may match other percussion. MP3/WAV variants and alternate copies may duplicate musical content: file count is not independent-song count.",
            "sampling":"At most 120 candidate beats per temporal third, per grid. Local tempo windows are 45 seconds centered in each third.","files":results});
        std::fs::write(&output,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    eprintln!("corpus report: {output}");
}

/// Broader negative-control sweep is diagnostic, not an accuracy benchmark.
#[test]
#[ignore = "opt-in synthetic negative-control sweep for rhythm evidence limits"]
fn inspect_rhythm_evidence_negative_controls() {
    let sr=22050;
    let mut results=Vec::new();
    for seconds in [2,4,8,15,60] {
        for seed in 1..=8_u64 {
            let mut state=seed;
            let m:Vec<_>=(0..sr as usize*seconds).map(|_|{
                state^=state<<13;state^=state>>7;state^=state<<17;
                ((state as f64/u64::MAX as f64)*2.0-1.0) as f32*0.05
            }).collect();
            let (bpm,_,beats,evidence)=detect_rhythm(&m,sr);
            results.push(json!({"kind":"stationary_noise","seconds":seconds,"seed":seed,"bpm":bpm,"map_beats":beats.len(),"rhythm_evidence":evidence}));
        }
        for frequency in [30.0,55.0,110.0,220.0,440.0,1000.0] {
            let m:Vec<_>=(0..sr as usize*seconds).map(|i|(0.4*(std::f64::consts::TAU*frequency*i as f64/sr as f64).sin()) as f32).collect();
            let (bpm,_,beats,evidence)=detect_rhythm(&m,sr);
            results.push(json!({"kind":"stationary_sine","seconds":seconds,"frequency_hz":frequency,"bpm":bpm,"map_beats":beats.len(),"rhythm_evidence":evidence}));
        }
    }
    let supported=results.iter().filter(|v|v["rhythm_evidence"]["grid_supported"]==true).count();
    let output=format!("/tmp/mixstudio-signal-probe-v{ANALYSIS_VERSION}.json");
    std::fs::write(&output,serde_json::to_vec_pretty(&json!({"cases":results,"supported_negative_controls":supported})).unwrap()).unwrap();
    eprintln!("negative-control sweep: {supported}/{} established grids; details {output}",results.len());
    assert_eq!(supported,0,"stationary negative controls must not establish beat grids; inspect diagnostic report");
}

/// A private manifest supplies source hashes and hand-marked waveform landmarks.
/// Export mode writes decoder-aligned windows before any reference is annotated.
/// Attack landmarks are not musical downbeat or tempo-octave annotations.
#[test]
#[ignore="requires a private manually annotated waveform-reference manifest"]
fn inspect_manual_waveform_references() {
    let manifest=std::env::var("MIXSTUDIO_REFERENCE_MANIFEST").expect("private JSON reference manifest");
    let cases:Vec<Value>=serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    assert!(!cases.is_empty());
    let export=std::env::var("MIXSTUDIO_REFERENCE_EXPORT").ok().map(PathBuf::from);
    if let Some(dir)=&export {std::fs::create_dir_all(dir).unwrap();}
    let mut report=Vec::new();
    for case in cases {
        let path=Path::new(case["path"].as_str().unwrap());
        let before=std::fs::metadata(path).unwrap();
        let hash=format!("{:x}",sha2::Sha256::digest(std::fs::read(path).unwrap()));
        assert_eq!(hash,case["source_sha256"].as_str().unwrap(),"source differs from annotated recording");
        let (audio,_,_)=decode(path).unwrap();
        assert_eq!(audio.l.len() as u64,case["frames"].as_u64().unwrap(),"annotation decoder timebase changed");
        let id=case["id"].as_str().unwrap();
        assert!(id.chars().all(|c|c.is_ascii_alphanumeric() || c=='-'));
        if let Some(dir)=&export {
            let lo=(case["start_seconds"].as_f64().unwrap()*audio.sr as f64).round() as usize;
            let hi=(case["end_seconds"].as_f64().unwrap()*audio.sr as f64).round() as usize;
            crate::engine::write_wav(&dir.join(format!("{id}.wav")),&audio.l[lo..hi],&audio.r[lo..hi],audio.sr).unwrap();
        } else {
            let landmarks:Vec<_>=case["attack_seconds"].as_array().unwrap().iter().map(|x|x.as_f64().unwrap()).collect();
            assert!(landmarks.len()>=4 && landmarks.windows(2).all(|w|w[1]>w[0]));
            let (bpm,offset,beats,_)=detect_rhythm(&rhythm_mono(&audio),audio.sr);
            assert!(!beats.is_empty());
            let matched:Vec<_>=landmarks.iter().map(|t|beats.iter().enumerate().min_by(|a,b|(a.1-t).abs().total_cmp(&(b.1-t).abs())).unwrap()).collect();
            let residuals:Vec<_>=matched.iter().zip(&landmarks).map(|((_,b),t)|*b-t).collect();
            let tolerance=case["tolerance_seconds"].as_f64().unwrap();
            assert!(residuals.iter().all(|x|x.abs()<=tolerance),"{id}: missed independently marked attacks: {residuals:?}");
            assert!(matched.windows(2).all(|w|w[1].0==w[0].0+1),"{id}: doubled or halved the marked pulse sequence");
            report.push(json!({"id":id,"bpm_estimate_not_label":bpm,"downbeat_estimate_not_label":offset,"attack_seconds":landmarks,
                "tolerance_seconds":tolerance,"residuals":distribution(residuals)}));
        }
        let after=std::fs::metadata(path).unwrap();assert_eq!(before.len(),after.len());assert_eq!(before.modified().unwrap(),after.modified().unwrap());
    }
    let output=std::env::var("MIXSTUDIO_REFERENCE_OUTPUT").unwrap_or_else(|_|"/tmp/mixstudio-manual-reference-results.json".into());
    std::fs::write(output,serde_json::to_vec_pretty(&json!({"analysis_version":ANALYSIS_VERSION,"cases":report,
        "interpretation":"Small waveform-landmark pilot marked visually before inspecting detected grids, using the app decoder timebase. Not listening-verified musical beat/downbeat labels or a broad accuracy benchmark."})).unwrap()).unwrap();
}
