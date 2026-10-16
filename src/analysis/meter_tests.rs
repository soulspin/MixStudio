//! Authored rhythm references: event timestamps and bar labels precede analysis.
//! These fixtures test explicit accents, not a universal definition of musical metre.
use super::*;

fn accented_reference(bpm:f64, pickup:usize, gain:f32, backbeat:f32)->(Vec<f32>,Vec<f64>,f64) {
    let sr=22050;let period=60.0/bpm;let lead=0.37;
    let times:Vec<_>=(0..160).map(|k|lead+k as f64*period).collect();
    let mut audio=vec![0.0;((times.last().unwrap()+period)*sr as f64) as usize];
    let mut seed=17_u32;
    for (k,&t) in times.iter().enumerate() {
        let phase=(k+4-pickup)%4;
        let amp=if phase==0 {1.0}else{0.55};
        let start=(t*sr as f64).round() as usize;
        for j in 0..(0.22*sr as f64).min(period*sr as f64) as usize {
            let dt=j as f64/sr as f64;
            let kick=amp*(std::f64::consts::TAU*(50.0+120.0*(-30.0*dt).exp())*dt).sin()*(-10.0*dt).exp();
            seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise=(seed as f64/u32::MAX as f64)*2.0-1.0;
            let snare=if phase%2==1 {backbeat as f64*noise*(-50.0*dt).exp()}else{0.0};
            audio[start+j]+=gain*(0.4*kick+snare) as f32;
        }
    }
    (audio,times,lead+pickup as f64*period)
}

#[test]
fn clearly_accented_pulses_are_not_forced_into_a_dance_tempo_range() {
    for expected in [70.0,80.0,140.0,180.0,190.0] {
        let (audio,times,downbeat)=accented_reference(expected,0,1.0,0.0);
        let (bpm,offset,beats)=detect_beats(&audio,22050);
        eprintln!("authored expected={expected} detected={bpm:.6} offset={offset:.5} first_downbeat={downbeat:.5} beats={}",beats.len());
        assert!((bpm-expected).abs()<0.02,"explicit quarter-note pulse {expected} was interpreted as {bpm}");
        let matched:Vec<_>=times[4..156].iter().map(|t|beats.iter().enumerate().min_by(|a,b|(a.1-t).abs().total_cmp(&(b.1-t).abs())).unwrap()).collect();
        assert!(matched.iter().zip(&times[4..156]).all(|((_,b),t)|(*b-t).abs()<0.005));
        assert!(matched.windows(2).all(|w|w[1].0==w[0].0+1),"quarter-note reference gained or lost markers");
    }
}

#[test]
fn accented_downbeat_survives_pickups_gain_and_louder_backbeats() {
    for pickup in 0..4 {
        for (gain,backbeat) in [(1.0,0.0),(1e-6,0.0),(1.0,0.8)] {
            let (audio,_,downbeat)=accented_reference(128.0,pickup,gain,backbeat);
            let (bpm,offset,_)=detect_beats(&audio,22050);
            let period=60.0/128.0;
            let bar_error=(offset-downbeat+2.0*period).rem_euclid(4.0*period)-2.0*period;
            eprintln!("authored pickup={pickup} gain={gain} snare={backbeat} bpm={bpm:.6} downbeat_error={bar_error:.6}");
            assert!((bpm-128.0).abs()<0.02,"backbeat changed the quarter-note tempo");
            assert!(bar_error.abs()<0.010,"wrong authored bar phase: pickup={pickup} gain={gain} snare={backbeat}, error={bar_error:.4}s");
        }
    }
}

#[test]
#[ignore="read-only tempo-family investigation"]
fn inspect_authored_tempo_families() {
    for expected in [70.0,80.0,90.0,128.0,140.0,180.0,190.0] {
        let (audio,_,_)=accented_reference(expected,0,1.0,0.0);
        let (o,fps)=onset_env(&audio,22050);
        let (bpm,off)=detect_bpm(&audio,22050);
        for b in [bpm/2.0,bpm,bpm*2.0] {
            if !(60.0..=200.0).contains(&b){continue;}
            let p=fps*60.0/b;
            let ph=(off*fps).rem_euclid(p);
            let mut phases:Vec<_>=(0..4).map(|k|comb(&o,p*4.0,ph+k as f64*p)).collect();phases.sort_by(|a,b|b.total_cmp(a));
            let ac=|lag:f64|{let k=lag as usize; let f=(lag-k as f64) as f32; (0..o.len()-k-1).map(|i|o[i] as f64*(o[i+k]*(1.0-f)+o[i+k+1]*f) as f64).sum::<f64>()/(o.len()-k) as f64};
            eprintln!("family target={expected} candidate={b:.3} ac={:.6} phase={phases:?} accent_gap={:.3}",ac(p)+0.5*ac(2.0*p)+0.25*ac(4.0*p),(phases[0]-phases[1])/phases[0].max(1e-20));
        }
    }
}

#[test]
fn source_evidence_distinguishes_accented_and_unaccented_bar_phase() {
    let mut margins=Vec::new();
    for gain in [1.0,1e-6] {
        let (audio,_,_)=accented_reference(128.0,2,gain,0.0);
        let (_,_,_,evidence)=detect_rhythm(&audio,22050);
        let json=serde_json::to_value(&evidence).unwrap();
        let margin=json["downbeat_bass_margin"].as_f64().expect("bar-phase evidence must be measured");
        assert!(margin>0.1,"clear authored bass accent was not distinguished");margins.push(margin);
    }
    assert!((margins[0]-margins[1]).abs()<0.01,"bar evidence changed with gain");
    let times:Vec<_>=(0..160).map(|k|0.37+k as f64*60.0/128.0).collect();
    let (mut audio,_,_)=accented_reference(128.0,0,1.0,0.0);
    // Remove the authored first-beat accent. Each pulse is now identical.
    for (_,t) in times.iter().enumerate().filter(|(k,_)|k%4==0) {
        let start=(t*22050.0).round() as usize;
        for v in &mut audio[start..start+(0.22*22050.0) as usize] {*v*=0.55;}
    }
    let (_,_,_,evidence)=detect_rhythm(&audio,22050);
    let json=serde_json::to_value(&evidence).unwrap();
    assert!(json["downbeat_bass_margin"].as_f64().unwrap()<0.03,"identical pulses invented a bar accent");
    assert!(evidence.warning().is_some(),"unaccented pulses looked like certain bar phase");
}

#[test]
fn eighth_note_hats_do_not_double_an_accented_quarter_note_pulse() {
    for bpm in [70.0,90.0,100.0] {
        let (mut audio,times,_)=accented_reference(bpm,1,1.0,0.0);
        let mut seed=41_u32;
        for &t in &times {
            let start=((t+30.0/bpm)*22050.0).round() as usize;
            for j in 0..(0.05*22050.0) as usize {
                seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);
                if start+j<audio.len() {audio[start+j]+=0.3*(seed as f64/u32::MAX as f64*2.0-1.0) as f32*(-80.0*j as f32/22050.0).exp();}
            }
        }
        let (actual,_,beats)=detect_beats(&audio,22050);
        assert!((actual-bpm).abs()<0.02,"eighth-note percussion doubled authored {bpm} BPM to {actual}");
        let matched:Vec<_>=times[4..156].iter().map(|t|beats.iter().enumerate().min_by(|a,b|(a.1-t).abs().total_cmp(&(b.1-t).abs())).unwrap()).collect();
        assert!(matched.iter().zip(&times[4..156]).all(|((_,b),t)|(*b-t).abs()<0.010),"hats moved the grid off the quarter-note attacks");
        assert!(matched.windows(2).all(|w|w[1].0==w[0].0+1),"hats gained or lost quarter-note grid markers");
    }
}

#[test]
fn a_short_supported_clip_does_not_hide_unknown_bar_phase() {
    let (mut audio,_,_)=accented_reference(128.0,0,1.0,0.0);
    audio.truncate((10.0*22050.0) as usize);
    let (_,_,_,evidence)=detect_rhythm(&audio,22050);
    assert!(evidence.grid_supported && (16..32).contains(&evidence.predicted_beats),"short clip fixture needs a supported pulse grid");
    assert!(evidence.warning().is_some(),"no measured bar phase must not look verified on a short clip");
}
