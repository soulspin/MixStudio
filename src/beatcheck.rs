//! Conservative source-attack measurements for explicitly requested beatgrid corrections.

use crate::{analysis::Audio, model::Track};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict { Stable, Drifting, Uncertain }
#[derive(Clone, Debug)]
pub struct Report {
    pub verdict: Verdict, pub offset_ms: f64, pub drift_ms: f64, pub jitter_ms: f64,
    pub correction_seconds: f64, pub pairs: usize, pub sampled: usize,
}
#[derive(Clone,Debug)]
pub struct BeatEvidence { pub beat:f64, pub reliable:bool, pub percussion:bool }

fn median(v: &mut [f64]) -> f64 { v.sort_by(f64::total_cmp); let n=v.len(); (v[n/2]+v[(n-1)/2])*0.5 }

// A local bass-attack envelope, filtered per channel to tolerate opposite stereo polarity.
// The rise test rejects continuous bass notes, where a slope alone is misleading.
fn attack(a:&Audio, center:f64, period:f64)->Option<f64> {
    if !center.is_finite() || period<=0.0 || a.sr<1000 { return None; }
    let sr=a.sr as f64; let radius=(period*0.20).min(0.10);
    let lo=((center-radius-0.08)*sr).max(0.0) as usize;
    let hi=((center+radius+0.025)*sr).max(0.0) as usize;
    if hi>=a.l.len().min(a.r.len()) || hi<=lo { return None; }
    let alpha=1.0-(-2.0*std::f64::consts::PI*150.0/sr).exp();
    let mut y1=[0.0;2]; let mut y2=[0.0;2]; let mut prev=[0.0;2]; let mut env=Vec::with_capacity(hi-lo);
    for i in lo..hi { let mut power=0.0;
        for (c,x) in [a.l[i],a.r[i]].iter().enumerate() {
            y1[c]+=alpha*(*x as f64-y1[c]); y2[c]+=alpha*(y1[c]-y2[c]);
            let q=(y2[c]-prev[c])*sr/(2.0*std::f64::consts::PI*60.0); prev[c]=y2[c];
            power+=y2[c]*y2[c]+q*q;
        } env.push((power*0.5).sqrt());
    }
    let lag=(sr*0.001).round().max(1.0) as usize;
    let begin=(((center-radius)*sr).max(0.0) as usize).saturating_sub(lo).max(lag);
    let end=(((center+radius)*sr).max(0.0) as usize).saturating_sub(lo).min(env.len());
    let i=(begin..end).max_by(|&i,&j|(env[i]-env[i-lag]).total_cmp(&(env[j]-env[j-lag])))?;
    let peak=env.iter().copied().fold(0.0,f64::max);
    if peak<0.002 || env[i]-env[i-lag]<peak*0.02 { return None; }
    let win=(sr*0.008).round().max(1.0) as usize;
    if i<win*2 || i+win>=env.len() { return None; }
    let pre=env[i-win*2..i-win].iter().sum::<f64>()/win as f64;
    let post=env[i..i+win].iter().sum::<f64>()/win as f64;
    if post<pre*1.8+0.001 { return None; }
    Some((lo+i) as f64/sr-center)
}

// Broadband high-pass onset energy catches snare/click attacks when the bass is weak.
// A sharp rise from a quiet local floor is required; continuous tones are rejected.
fn percussion_attack(a:&Audio,center:f64,period:f64)->Option<f64> {
    if !center.is_finite() || !period.is_finite() || period<=0.0 || a.sr<1000 { return None; }
    let sr=a.sr as f64; let radius=(period*0.20).min(0.10);
    let lo=((center-radius-0.060)*sr).max(0.0) as usize; let hi=((center+radius+0.020)*sr).max(0.0) as usize;
    if hi>=a.l.len().min(a.r.len()) || hi<=lo { return None; }
    let hp=1.0-(-2.0*std::f64::consts::PI*700.0/sr).exp(); let smooth=1.0-(-1.0/(sr*0.001)).exp();
    let mut low=[0.0;2]; let mut energy=0.0; let mut env=Vec::with_capacity(hi-lo);
    for i in lo..hi { let mut p=0.0; for (c,x) in [a.l[i],a.r[i]].into_iter().enumerate() { low[c]+=hp*(x as f64-low[c]); p+=(x as f64-low[c]).powi(2); }
        energy+=smooth*(p*0.5-energy); env.push(energy.sqrt());
    }
    let lag=(sr*0.002).round().max(1.0) as usize;
    let begin=(((center-radius)*sr).max(0.0) as usize).saturating_sub(lo).max(lag);
    let end=(((center+radius)*sr).max(0.0) as usize).saturating_sub(lo).min(env.len());
    let i=(begin..end).max_by(|&i,&j|(env[i]-env[i-lag]).total_cmp(&(env[j]-env[j-lag])))?;
    let peak=env.iter().copied().fold(0.0,f64::max); let win=(sr*0.008).round().max(1.0) as usize;
    if peak<0.008 || i<win*2 || i+win>=env.len() || env[i]-env[i-lag]<peak*0.25 { return None; }
    let pre=env[i-win*2..i-win].iter().sum::<f64>()/win as f64;
    let post=env[i..i+win].iter().sum::<f64>()/win as f64;
    if post<pre*2.5+0.004 { return None; }
    Some((lo+i) as f64/sr-center)
}
fn combined_attack(a:&Audio,center:f64,period:f64)->Option<(f64,bool)> {
    let bass=attack(a,center,period); let high=percussion_attack(a,center,period);
    match (bass,high) {
        (Some(b),Some(h)) if (b-h).abs()>0.015=>None, // conflicting evidence stays uncertain
        (Some(b),_)=>Some((b,false)), (None,Some(h))=>Some((h,true)), _=>None,
    }
}

/// Read-only evidence for the ignored local-corpus diagnostic; residuals are not ground truth.
#[cfg(test)]
pub(crate) fn diagnostic_attacks(a: &Audio, center: f64, period: f64) -> (Option<f64>, Option<f64>, Option<(f64, bool)>) {
    (attack(a, center, period), percussion_attack(a, center, period), combined_attack(a, center, period))
}

#[cfg(test)]
pub fn measure(a:&Track,aa:&Audio,b:&Track,ab:&Audio,start:f64,end:f64,bpm_at:impl Fn(f64)->f64)->Result<Report,String> {
    inspect(a,aa,b,ab,start,end,bpm_at).0
}

/// Compare paired bass attacks in the shared mix grid. Musical intent is advisory:
/// only consistent evidence permits an explicitly requested constant correction.
pub fn inspect(a:&Track, aa:&Audio, b:&Track, ab:&Audio, start:f64, end:f64, bpm_at:impl Fn(f64)->f64)->(Result<Report,String>,Vec<BeatEvidence>) {
    let mut evidence=vec![];
    let report=(|| {
    if !start.is_finite() || !end.is_finite() || end-start<12.0 { return Err("Use a longer overlap with at least 12 clear paired drum attacks.".into()); }
    if (a.beat_mult-1.0).abs()>1e-9 || (b.beat_mult-1.0).abs()>1e-9 { return Err("Attack comparison currently needs a 1:1 beat ratio on both tracks.".into()); }
    let ga=a.grid(); let gb=b.grid(); let mut pairs=vec![]; let mut source=vec![]; let mut sampled=0;
    let stride=((end-start)/64.0).ceil().max(1.0) as usize;
    for k in ((start.ceil() as i64+1)..(end.floor() as i64-1)).step_by(stride) {
        let beat=k as f64; sampled+=1;
        evidence.push(BeatEvidence{beat,reliable:false,percussion:false});
        if ga.cuts().chain(gb.cuts()).any(|c|(c-beat).abs()<0.5) { continue; }
        let pa=ga.region_sec(beat+0.5,beat)-ga.region_sec(beat-0.5,beat);
        let pb=gb.region_sec(beat+0.5,beat)-gb.region_sec(beat-0.5,beat);
        let bpm=bpm_at(beat); if !bpm.is_finite() || bpm<=0.0 { continue; }
        if let (Some((x,ha)),Some((y,hb)))=(combined_attack(aa,ga.sec(beat),pa),combined_attack(ab,gb.sec(beat),pb)) {
            let ev=evidence.last_mut().unwrap(); ev.reliable=true; ev.percussion=ha||hb;
            let delta=y/pb-x/pa; pairs.push((beat,delta*60000.0/bpm)); source.push(delta*pb);
        }
    }
    if pairs.len()<12 || pairs.len()*2<sampled || pairs.last().zip(pairs.first()).is_none_or(|(last,first)|last.0-first.0<(end-start)*0.6) { return Err(format!("Not enough consistent rhythm attacks ({}/{} beats). Listen and adjust manually.",pairs.len(),sampled)); }
    let mut slopes=vec![];
    for i in 0..pairs.len() { for j in i+1..pairs.len() { if pairs[j].0-pairs[i].0>=4.0 { slopes.push((pairs[j].1-pairs[i].1)/(pairs[j].0-pairs[i].0)); } } }
    if slopes.is_empty() { return Err("Not enough attack coverage to measure drift.".into()); }
    let slope=median(&mut slopes); let mid=(pairs[0].0+pairs.last().unwrap().0)*0.5;
    let mut centered:Vec<_>=pairs.iter().map(|(x,y)|y-slope*(x-mid)).collect(); let offset=median(&mut centered);
    let mut deviations:Vec<_>=pairs.iter().map(|(x,y)|(y-offset-slope*(x-mid)).abs()).collect();
    let jitter=median(&mut deviations); let tail=deviations[(deviations.len()*9/10).min(deviations.len()-1)]; let drift=slope*(pairs.last().unwrap().0-pairs[0].0);
    let correction=median(&mut source);
    let mut source_spread:Vec<_>=source.iter().map(|s|(s-correction).abs()).collect(); source_spread.sort_by(f64::total_cmp);
    let source_tail=source_spread[(source_spread.len()*9/10).min(source_spread.len()-1)];
    let verdict=if jitter>6.0 || tail>15.0 || (drift.abs()<=8.0 && source_tail>0.008) { Verdict::Uncertain } else if drift.abs()>8.0 { Verdict::Drifting } else { Verdict::Stable };
    Ok(Report {verdict,offset_ms:offset,drift_ms:drift,jitter_ms:jitter,correction_seconds:correction,pairs:pairs.len(),sampled})
    })(); (report,evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analysis::Audio,model::Track};
    pub fn track() -> Track { serde_json::from_str(r#"{"path":"test.wav","title":"Test","artist":"Test","bpm":120.0,"offset":0.25,"key":"8A","duration":24.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap() }
    fn kicks(shift:f64,drift:f64)->Audio {
        let sr=16000;let times:Vec<_>=(0..46).map(|i|0.25+i as f64*0.5+shift+drift*i as f64/45.0).collect();
        let (l,r)=crate::testgen::synth_kicks(&times,24.0,sr);Audio{sr,l,r}
    }
    #[test]
    #[ignore = "requires MIXSTUDIO_DIAGNOSTIC_PROJECT pointing to local source audio"]
    fn inspect_saved_overlap_without_mutating_project() {
        let path=std::env::var("MIXSTUDIO_DIAGNOSTIC_PROJECT").unwrap();
        let mix:crate::model::Mix=serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let (o,i,s,e)=mix.transitions()[0]; let a=&mix.tracks[o]; let b=&mix.tracks[i];
        let (aa,_)=crate::analysis::load(std::path::Path::new(&a.path)).unwrap();
        let (ab,_)=crate::analysis::load(std::path::Path::new(&b.path)).unwrap();
        let result=measure(a,&aa,b,&ab,s,e,|beat|mix.bpm_at(beat)); eprintln!("Saved transition: {result:?}");
    }
    #[test]
    fn percussion_without_bass_is_measured_and_weak_beats_are_exposed() {
        let a=track();
        let make=|shift:f64| { let sr=16000; let mut l=vec![0.0;sr*24];
            for k in 0..46 { if k%7==0 { continue; } let at=((0.25+k as f64*0.5+shift)*sr as f64) as usize;
                for j in 0..400 { l[at+j]=0.7*(j as f32*2.0*std::f32::consts::PI*2400.0/sr as f32).sin()*(-(j as f32)/90.0).exp(); }
            }
            for (j,x) in l.iter_mut().enumerate() { *x+=0.4*(j as f32*2.0*std::f32::consts::PI*60.0/sr as f32).sin(); }
            Audio{sr:sr as u32,r:l.clone(),l}
        };
        let (report,evidence)=inspect(&a,&make(0.0),&a,&make(0.018),2.0,42.0,|_|120.0);
        let report=report.unwrap(); assert_eq!(report.verdict,Verdict::Stable,"{report:?}"); assert!((report.offset_ms-18.0).abs()<2.0);
        assert!(evidence.iter().any(|e|!e.reliable)); assert!(evidence.iter().any(|e|e.reliable && e.percussion));
    }
    #[test]
    fn stable_offset_has_signed_source_correction_and_improves_alignment() {
        for shift in [-0.024,0.018] {
            let a=track();let mut b=track();let aa=kicks(0.0,0.0);let ab=kicks(shift,0.0);
            let report=measure(&a,&aa,&b,&ab,2.0,42.0,|_|120.0).unwrap();
            assert_eq!(report.verdict,Verdict::Stable,"{report:?}");
            assert!((report.offset_ms-shift*1000.0).abs()<2.0,"{report:?}");
            b.adjust_grid(crate::model::GridScope::All,16.0,report.correction_seconds).unwrap();
            let after=measure(&a,&aa,&b,&ab,2.0,42.0,|_|120.0).unwrap();assert!(after.offset_ms.abs()<2.0,"{after:?}");
        }
    }
    #[test]
    fn drift_is_reported_without_enabling_a_constant_shift() {
        let a=track();let aa=kicks(0.0,0.0);let ab=kicks(0.0,0.040);
        let r=measure(&a,&aa,&a,&ab,2.0,42.0,|_|120.0).unwrap();
        assert_eq!(r.verdict,Verdict::Drifting,"{r:?}");assert!(r.drift_ms>25.0,"{r:?}");
    }
    #[test]
    fn silence_sustained_bass_and_short_overlaps_do_not_produce_a_correction() {
        let a=track();let aa=kicks(0.0,0.0);let silence=Audio{sr:16000,l:vec![0.0;16000*24],r:vec![0.0;16000*24]};
        assert!(measure(&a,&aa,&a,&silence,2.0,42.0,|_|120.0).is_err());
        let sine:Vec<_>=(0..16000*24).map(|i|0.5*(i as f32*2.0*std::f32::consts::PI*60.0/16000.0).sin()).collect();
        let tone=Audio{sr:16000,l:sine.clone(),r:sine};assert!(measure(&a,&aa,&a,&tone,2.0,42.0,|_|120.0).is_err());
        assert!(measure(&a,&aa,&a,&aa,2.0,5.0,|_|120.0).is_err());
    }
    #[test]
    fn repeated_disagreement_cannot_hide_behind_zero_median_scatter() {
        let a=track(); let aa=kicks(0.0,0.0);
        let times:Vec<_>=(0..46).map(|i|0.25+i as f64*0.5+if i%3==0{-0.040}else{0.018}).collect();
        let (l,r)=crate::testgen::synth_kicks(&times,24.0,16000); let ab=Audio{sr:16000,l,r};
        let report=measure(&a,&aa,&a,&ab,2.0,42.0,|_|120.0).unwrap();
        assert_eq!(report.verdict,Verdict::Uncertain,"{report:?}");
    }
    #[test]
    fn scattered_offbeat_attacks_stay_advisory() {
        let a=track();let aa=kicks(0.0,0.0);let times:Vec<_>=(0..46).map(|i|0.25+i as f64*0.5+if i%3==0{0.05}else if i%3==1{-0.04}else{0.0}).collect();
        let(l,r)=crate::testgen::synth_kicks(&times,24.0,16000);let b=Audio{sr:16000,l,r};
        let r=measure(&a,&aa,&a,&b,2.0,42.0,|_|120.0).unwrap();assert_eq!(r.verdict,Verdict::Uncertain,"{r:?}");
    }
}
