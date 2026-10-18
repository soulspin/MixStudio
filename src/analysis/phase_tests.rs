use super::*;

#[test]
fn steady_clock_survives_a_breakdown_with_offbeat_percussion() {
    let sr=16000;let period=60.0/126.0;let count=256;
    let beats:Vec<_>=(0..count).map(|i|0.3+i as f64*period).collect();
    let times:Vec<_>=beats.iter().enumerate().filter(|(i,_)|!(80..144).contains(i)).map(|(_,t)|*t).collect();
    let (mut l,mut r)=crate::testgen::synth_kicks(&times,123.0,sr);
    for t in &beats[80..144] {
        let at=((t+period*0.5)*sr as f64) as usize;
        for j in 0..1000 {
            let x=0.35*(j as f32*std::f32::consts::TAU*1700.0/sr as f32).sin()*(-(j as f32)/110.0).exp();
            l[at+j]+=x;r[at+j]+=x;
        }
    }
    let (_,_,grid,_)=detect_rhythm(&rhythm_mono(&Audio{sr,l,r}),sr);
    assert!(grid.len()>240);
    let error=beats[88..136].iter().map(|t|grid.iter().map(|b|(b-t).abs()).fold(f64::INFINITY,f64::min)).fold(0.0,f64::max);
    assert!(error<0.012,"offbeat percussion pulled a steady clock away by {error:.4}s");
}
