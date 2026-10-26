//! Reject a DP phase excursion only when widely distributed source attacks
//! independently support one steady clock. This is not a tempo-change detector.
/// A replacement beat grid and the source attacks (beat index, seconds) that support it.
type Clock=(Vec<f64>,Vec<(f64,f64)>);
pub(super) fn bridge(beats:&[f64],observed:&[(f64,f64)],bpm:f64)->Option<Clock> {
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

/// The DP tracker can also drift smoothly off a steady track's kicks by tens of
/// milliseconds without skipping a beat. Attacks searched near its own beats then
/// follow the drift, so `bridge` cannot see it. Search attacks on a straight clock
/// directly and adopt the clock only when it is supported throughout the source,
/// the tracker departs from it, and no run of attacks corroborates the departure.
pub(super) fn steady_clock(m:&[f32],sr:u32,beats:&[f64],bpm:f64)->Option<Clock> {
    if beats.len()<96 || !bpm.is_finite() || bpm<=0.0 || sr==0 {return None;}
    let ch=crate::beatcheck::Ch{l:m,r:m,sr}; let duration=m.len() as f64/sr as f64;
    let period=60.0/bpm;
    let mut phases:Vec<_>=beats.iter().enumerate().map(|(k,t)|t-k as f64*period).collect();
    phases.sort_by(f64::total_cmp);
    let (mut p,mut c)=(period,phases[phases.len()/2]);
    let first=|p:f64,c:f64|((-0.02-c)/p).ceil() as i64;
    // Fit kicks when there are enough of them; claps and hats sit a few ms away and would bias the clock.
    let kicks_only={let mut k=first(p,c);let mut hits=0;while c+k as f64*p<duration {hits+=crate::beatcheck::bass_attack(ch,c+k as f64*p,p).is_some() as usize;k+=1;}hits>=96};
    let find=|t:f64,p:f64|if kicks_only {crate::beatcheck::bass_attack(ch,t,p)} else {crate::beatcheck::combined_attack(ch,t,p).map(|x|x.0)};
    let mut attacks=vec![];
    // Two passes: the second re-centres the search window on the fitted clock.
    for pass in 0..2 {
        attacks.clear();
        let mut k=first(p,c);
        while c+k as f64*p<duration { let t=c+k as f64*p; if let Some(x)=find(t,p) {attacks.push((k as f64,t+x));} k+=1; }
        if attacks.len()<64 {return None;}
        let mut ph:Vec<_>=attacks.iter().map(|&(k,t)|t-k*p).collect(); ph.sort_by(f64::total_cmp); let mid=ph[ph.len()/2];
        let mut points:Vec<_>=attacks.iter().copied().filter(|&(k,t)|(t-k*p-mid).abs()<0.010).collect();
        for _ in 0..3 {
            if points.len()<32 {return None;}
            let mx=points.iter().map(|q|q.0).sum::<f64>()/points.len() as f64; let my=points.iter().map(|q|q.1).sum::<f64>()/points.len() as f64;
            let xx=points.iter().map(|q|(q.0-mx).powi(2)).sum::<f64>(); if xx<=0.0 {return None;}
            p=points.iter().map(|q|(q.0-mx)*(q.1-my)).sum::<f64>()/xx; c=my-p*mx;
            if (p/period-1.0).abs()>0.003 {return None;}
            points=attacks.iter().copied().filter(|&(k,t)|(t-c-k*p).abs()<=0.008).collect();
        }
        if pass==0 {
            // Kicks define the beat: hi-hats on the offbeat can fit the same clock half a beat away.
            let bass=|shift:f64|{let mut k=first(p,c+shift);let mut hits=0;while c+shift+k as f64*p<duration {hits+=crate::beatcheck::bass_attack(ch,c+shift+k as f64*p,p).is_some() as usize;k+=1;}hits};
            let (on,off)=(bass(0.0),bass(p*0.5));
            if off>=32 && off*2>on*3 { c+=p*0.5; }
            continue;
        }
        let n=((duration-c)/p).floor() as i64-first(p,c)+1; let n=n.max(1) as usize;
        // Supported throughout: most attacks agree, and every third of the file contributes.
        if points.len()<64 || points.len()*10<attacks.len()*6 || points.len()*10<n*3 {return None;}
        let lo=first(p,c) as f64; let span=n as f64;
        for third in 0..3 {
            let count=points.iter().filter(|q|q.0-lo>=span*third as f64/3.0 && q.0-lo<span*(third+1) as f64/3.0).count();
            if count<8 || count*12<n {return None;}
        }
        // Leave a tracker that already follows the clock alone.
        let nearest=|t:f64|beats.iter().map(|b|(b-t).abs()).fold(f64::INFINITY,f64::min);
        let mut dev:Vec<_>=(0..n).map(|i|{let t=c+(lo+i as f64)*p; let k=((t-beats[0])/period).round().clamp(0.0,(beats.len()-1) as f64) as usize; beats[k.saturating_sub(1)..(k+2).min(beats.len())].iter().map(|b|(b-t).abs()).fold(f64::INFINITY,f64::min)}).collect();
        dev.sort_by(f64::total_cmp); if dev[dev.len()*9/10]<=0.015 {return None;}
        // Genuine tempo motion: a run of off-clock attacks that the tracker follows.
        let mut run=0;
        for &(k,t) in &attacks {
            let off=(t-c-k*p).abs();
            if off>0.015 && nearest(t)<=0.010 { run+=1; if run>=8 {return None;} } else if off<=0.008 { run=0; }
        }
        let clock:Vec<_>=(0..n).map(|i|c+(lo+i as f64)*p).collect();
        // Adopt the clock only when it demonstrably lands on more kicks than the tracker.
        let on_kick=|grid:&[f64]|grid.windows(2).filter(|w|crate::beatcheck::bass_attack(ch,w[0],w[1]-w[0]).is_some_and(|x|x.abs()<=0.010)).count();
        let (old,new)=(on_kick(beats),on_kick(&clock));
        if new<old+(old/20).max(8) {return None;}
        return Some((clock,points.into_iter().map(|(k,t)|(k-lo,t)).collect()));
    }
    None
}

/// A steady tracker can lock onto offbeat hi-hats, or sit a constant few tens of ms
/// beside soft kicks, for the whole file. Kicks define the beat in dance music: try a
/// half-beat move and a shift by the median kick offset, and keep a candidate only
/// when it lands clearly more kicks (half as many again). Good grids cannot qualify.
pub(super) fn kick_phase(m:&[f32],sr:u32,beats:&[f64])->Option<Vec<f64>> {
    if beats.len()<32 || sr==0 {return None;}
    let ch=crate::beatcheck::Ch{l:m,r:m,sr};
    let on_kick=|grid:&[f64]|grid.windows(2).filter(|w|crate::beatcheck::bass_attack(ch,w[0],w[1]-w[0]).is_some_and(|x|x.abs()<=0.010)).count();
    let better=|grid:&[f64],score:usize|{let s=on_kick(grid);(s>=32 && s*2>=score*3).then_some(s)};
    let mut best=beats.to_vec(); let mut score=on_kick(beats); let mut changed=false;
    let half:Vec<_>=best.iter().enumerate().map(|(i,t)|{let p=if i+1<best.len() {best[i+1]-t} else {t-best[i-1]};t+p*0.5}).collect();
    if let Some(s)=better(&half,score) { best=half; score=s; changed=true; }
    let mut offsets:Vec<_>=best.windows(2).filter_map(|w|crate::beatcheck::bass_attack(ch,w[0],w[1]-w[0])).collect();
    if offsets.len()>=64 {
        offsets.sort_by(f64::total_cmp); let shift=offsets[offsets.len()/2];
        let moved:Vec<_>=best.iter().map(|t|t+shift).collect();
        if shift.abs()>0.008 && better(&moved,score).is_some() { best=moved; changed=true; }
    }
    changed.then_some(best)
}
#[cfg(test)]
mod tests {
    use super::*;
    // Kicks on the beat plus noisy hi-hats on every offbeat, mono at 22.05 kHz.
    fn kicks_and_hats(kicks:&[f64])->(Vec<f32>,u32) {
        let sr=22050; let (mut m,_)=crate::testgen::synth_kicks(kicks,(kicks[kicks.len()-1]+1.0) as f32,sr);
        let mut seed=1u32;
        for w in kicks.windows(2) { let s0=((w[0]+w[1])*0.5*sr as f64) as usize;
            for i in 0..(0.03*sr as f64) as usize { seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);
                if let Some(x)=m.get_mut(s0+i) { *x+=0.35*(-(i as f32)/(0.006*sr as f32)).exp()*((seed>>16) as f32/32768.0-1.0); } } }
        (m,sr)
    }
    fn on_kicks(clock:&[f64],kicks:&[f64])->f64 {
        clock.iter().filter(|t|**t>kicks[0]-0.1 && **t<kicks[kicks.len()-1]+0.1).map(|t|kicks.iter().map(|k|(k-t).abs()).fold(f64::INFINITY,f64::min)).fold(0.0,f64::max)
    }
    #[test]
    fn steady_clock_replaces_a_tracker_that_wanders_off_the_kicks() {
        let kicks:Vec<_>=(0..300).map(|k|0.3+k as f64*0.48).collect(); let (m,sr)=kicks_and_hats(&kicks);
        for base in [0.0,0.24] { // on the beat, and wandering around the offbeat hi-hats
            let beats:Vec<_>=kicks.iter().enumerate().map(|(k,t)|t+base+0.09*(k as f64/30.0).sin()).collect();
            let (clock,inliers)=steady_clock(&m,sr,&beats,125.0).expect("steady kicks should certify a straight clock");
            assert!(on_kicks(&clock,&kicks)<0.004,"clock {:.1} ms off the kicks (base {base})",on_kicks(&clock,&kicks)*1000.0);
            assert!(inliers.len()>=200);
        }
    }
    #[test]
    fn kick_phase_moves_offbeat_and_offset_grids_onto_the_kicks_only() {
        let kicks:Vec<_>=(0..200).map(|k|0.3+k as f64*0.48).collect(); let (m,sr)=kicks_and_hats(&kicks);
        assert!(kick_phase(&m,sr,&kicks).is_none(),"a grid on the kicks must stay");
        for shift in [0.24,0.05,-0.045] {
            let grid:Vec<_>=kicks.iter().map(|t|t+shift).collect();
            let moved=kick_phase(&m,sr,&grid).unwrap_or_else(||panic!("grid {shift} s off was kept"));
            assert!(on_kicks(&moved,&kicks)<0.004,"shift {shift}: still {:.1} ms off",on_kicks(&moved,&kicks)*1000.0);
        }
    }
    #[test]
    fn steady_clock_leaves_a_good_tracker_and_real_tempo_changes_alone() {
        let kicks:Vec<_>=(0..300).map(|k|0.3+k as f64*0.48).collect(); let (m,sr)=kicks_and_hats(&kicks);
        assert!(steady_clock(&m,sr,&kicks,125.0).is_none(),"an accurate tracker must be kept");
        let jump=crate::testgen::jump_times(125.0,150,128.0,150,0.3); let (m,sr)=kicks_and_hats(&jump);
        assert!(steady_clock(&m,sr,&jump,125.0).is_none(),"a supported tempo change must not be flattened");
        let late:Vec<_>=(0..300).map(|k|{let k=k as f64;0.3+k*0.48+(k-200.0).max(0.0)*0.006}).collect(); let (m,sr)=kicks_and_hats(&late);
        assert!(steady_clock(&m,sr,&late,125.0).is_none(),"a late tempo change followed by the tracker must be kept");
    }
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
