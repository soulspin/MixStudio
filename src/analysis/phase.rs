//! Reject a DP phase excursion only when widely distributed source attacks
//! independently support one steady clock. This is not a tempo-change detector.
pub(super) fn bridge(beats:&[f64],observed:&[(f64,f64)],bpm:f64)->Option<(Vec<f64>,Vec<(f64,f64)>)> {
    if beats.len()<96 || observed.len()<64 || !bpm.is_finite() || bpm<=0.0 {return None;}
    let period=60.0/bpm;
    let mut phases:Vec<_>=observed.iter().map(|&(k,t)|t-k*period).collect();
    phases.sort_by(f64::total_cmp);let phase=phases[phases.len()/2];
    let mut points:Vec<_>=observed.iter().copied().filter(|&(k,t)|(t-k*period-phase).abs()<0.025).collect();
    let fit=|points:&[(f64,f64)]|->Option<(f64,f64)> {
        if points.len()<32 {return None;}
        let mx=points.iter().map(|p|p.0).sum::<f64>()/points.len() as f64;
        let my=points.iter().map(|p|p.1).sum::<f64>()/points.len() as f64;
        let xx=points.iter().map(|p|(p.0-mx).powi(2)).sum::<f64>();
        if xx<=0.0 {return None;}
        let p=points.iter().map(|v|(v.0-mx)*(v.1-my)).sum::<f64>()/xx;
        Some((p,my-p*mx))
    };
    for _ in 0..3 {
        let (p,c)=fit(&points)?;
        if (p/period-1.0).abs()>0.003 {return None;}
        points=observed.iter().copied().filter(|&(k,t)|(t-c-k*p).abs()<=0.010).collect();
    }
    let (p,c)=fit(&points)?;
    if (p/period-1.0).abs()>0.003 || points.len()<64 || points.len()*5<observed.len()*3 || points.len()*5<beats.len()*2 {return None;}
    let span=(beats.len()-1) as f64;
    if points.last()?.0-points.first()?.0<span*0.80 {return None;}
    // No single drop/verse can certify the clock for the whole source.
    for third in 0..3 {
        let count=points.iter().filter(|p|p.0>=span*third as f64/3.0 && p.0<=span*(third+1) as f64/3.0).count();
        if count<8 || count*12<beats.len() {return None;}
    }
    // Genuine tempo motion can occupy only a minority of the file and still
    // return to the original phase. Do not override a ramp corroborated by
    // nearby source attacks (including halftime kicks); DP predictions across missing attacks are
    // not such evidence.
    for w in observed.windows(5) {
        if !w.windows(2).all(|v|v[1].0>v[0].0 && v[1].0-v[0].0<=4.0) {continue;}
        let r:Vec<_>=w.iter().map(|&(k,t)|t-c-k*p).collect();
        let motion=r[4]-r[0];
        if motion.abs()>p*0.025 && r.iter().any(|v|v.abs()>p*0.08)
            && r.windows(2).zip(w.windows(2)).all(|(v,k)|(v[1]-v[0])/(k[1].0-k[0].0)*motion.signum()>p*0.002) {return None;}
    }
    // Small natural timing variation is left alone. Require a sustained large
    // phase excursion, rather than flattening every apparently steady source.
    if !beats.windows(4).enumerate().any(|(k,w)|w.iter().enumerate().all(|(j,t)|(t-(c+(k+j) as f64*p)).abs()>p*0.20)) {return None;}
    Some(((0..beats.len()).map(|k|c+k as f64*p).collect(),points))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_drift_and_tempo_jumps_are_not_flattened() {
        for jump in [false,true] {
            let beats:Vec<_>=(0..256).map(|i| {let k=i as f64;0.3+k*0.48+if jump {(k-128.0).max(0.0)*0.035}else{0.00002*(k-128.0).powi(2)}}).collect();
            let observed:Vec<_>=beats.iter().enumerate().map(|(k,t)|(k as f64,*t)).collect();
            assert!(bridge(&beats,&observed,125.0).is_none(),"flattened real timing variation");
        }
    }
    #[test]
    fn temporary_and_late_supported_tempo_changes_are_not_flattened() {
        for late in [false,true] {
            let beats:Vec<_>=(0..256).map(|i| {let k=i as f64;0.2+0.5*k+if late {(k-225.0).max(0.0)*0.012}else{0.01*(24.0-(k-120.0).abs()).max(0.0)}}).collect();
            let observed:Vec<_>=beats.iter().enumerate().map(|(k,t)|(k as f64,*t)).collect();
            assert!(bridge(&beats,&observed,120.0).is_none(),"flattened supported temporary/late tempo change (late={late})");
        }
    }
    #[test]
    fn sparse_attacks_still_protect_real_tempo_motion() {
        let beats:Vec<_>=(0..256).map(|i| {let k=i as f64;0.2+0.5*k+0.015*(16.0-(k-120.0).abs()).max(0.0)}).collect();
        let observed:Vec<_>=beats.iter().enumerate().step_by(2).map(|(k,t)|(k as f64,*t)).collect();
        assert!(bridge(&beats,&observed,120.0).is_none(),"flattened true tempo motion supported by halftime kicks");
    }
    #[test]
    fn one_supported_passage_cannot_certify_an_entire_song() {
        let beats:Vec<_>=(0..256).map(|k|0.2+k as f64*0.5+if k>160 {0.2}else{0.0}).collect();
        let observed:Vec<_>=(0..160).map(|k|(k as f64,0.2+k as f64*0.5)).collect();
        assert!(bridge(&beats,&observed,120.0).is_none());
    }
}
