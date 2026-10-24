//! Manual source attacks assigned to two mix beats.

use crate::{beatmap::BeatMap,model::Track};
use std::sync::Arc;

#[derive(Clone,Copy,Debug)]
pub struct Anchor { pub beat:f64, pub source:f64 }
#[derive(Clone,Copy,Debug)]
pub struct Adjustment { pub bpm:f64, pub percent:f64 }

/// Preserve the existing grid's local shape while fitting both explicit source attacks.
/// The original clip is never mutated, including validation failures and previews.
pub fn anchored(t:&Track,a:Anchor,b:Anchor)->Result<(Track,Adjustment),String> {
    if !t.sections.is_empty() || (t.beat_mult-1.0).abs()>1e-9 { return Err("Use an uncut source clip with a 1:1 beat ratio.".into()); }
    if [a.beat,b.beat,a.source,b.source,t.bpm].iter().any(|x|!x.is_finite()) || t.bpm<=0.0 { return Err("Choose two finite source attacks.".into()); }
    if b.beat-a.beat<4.0 || b.source<=a.source { return Err("A must precede B by at least four beats. Use distant, clear attacks.".into()); }
    if a.beat<t.vis_start(0.0) || b.beat>t.end(0.0) || a.source<t.trim_in || b.source>t.duration-t.trim_out { return Err("Both anchors must be inside the visible source clip.".into()); }
    let g=t.grid_raw(); let old_a=g.sec(a.beat); let old_b=g.sec(b.beat);
    let scale=(b.source-a.source)/(old_b-old_a); let bpm=t.bpm/scale;
    if !scale.is_finite() || !(0.8..=1.25).contains(&scale) || !(40.0..=250.0).contains(&bpm) { return Err("The proposed tempo change is too large. Check the beat numbers.".into()); }
    let transform=|s:f64|a.source+(s-old_a)*scale;
    let mut map=if g.dynamic() { (**t.cur_map().as_ref().unwrap()).clone() } else { BeatMap::from_grid(&g,t.start_beat,t.duration,t.bpm) };
    for s in &mut map.beats { *s=transform(*s); } map.bpm=if g.dynamic() {map.bpm/scale} else {bpm};
    let mut fixed=t.clone(); fixed.offset=transform(t.offset); fixed.bpm=bpm; fixed.map=Some(Arc::new(map)); fixed.grid_mode=4; fixed.normalize_grid();
    Ok((fixed,Adjustment{bpm,percent:(1.0/scale-1.0)*100.0}))
}

/// Apply a measured correction line as two anchors, fixing phase and a uniform tempo error together.
pub fn measured(t:&Track,line:[(f64,f64);2])->Result<(Track,Adjustment),String> {
    let g=t.grid_raw(); let [(a,ca),(b,cb)]=line;
    anchored(t,Anchor{beat:a,source:g.sec(a)+ca},Anchor{beat:b,source:g.sec(b)+cb})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track()->crate::model::Track { serde_json::from_str(r#"{"path":"test.wav","title":"Test","artist":"Test","bpm":120.0,"offset":0.25,"key":"8A","duration":40.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap() }
    #[test]
    fn two_attacks_correct_phase_and_drift_and_survive_save() {
        let t=track(); let a=Anchor{beat:8.0,source:4.270}; let b=Anchor{beat:40.0,source:20.310};
        let (fixed,p)=anchored(&t,a,b).unwrap(); assert!((p.bpm-120.0/1.0025).abs()<1e-8);
        assert!((fixed.sec_at(a.beat)-a.source).abs()<1e-9); assert!((fixed.sec_at(b.beat)-b.source).abs()<1e-9);
        assert_eq!(fixed.grid_mode,4); assert!(fixed.grid().dynamic());
        let saved:crate::model::Track=serde_json::from_str(&serde_json::to_string(&fixed).unwrap()).unwrap();
        assert!((saved.sec_at(40.0)-b.source).abs()<1e-9); assert_eq!(t.bpm,120.0);
    }
    #[test]
    fn affine_fit_preserves_fractional_anchor_and_internal_map_ratio() {
        for bpm in [60.0,120.0,240.0] {
            let mut t=track(); t.start_beat=0.37; t.offset=0.361; t.bpm=bpm;
            t.map=Some(Arc::new(BeatMap{beats:(0..180).map(|i|0.25+i as f64*0.5+(i as f64*0.4).sin()*0.007).collect(),bpm:120.0}));
            let a=Anchor{beat:8.0,source:t.sec_at(8.0)+0.020};
            let b=Anchor{beat:32.0,source:t.sec_at(32.0)+0.040};
            let (fixed,_)=anchored(&t,a,b).unwrap();
            assert!((fixed.sec_at(a.beat)-a.source).abs()<1e-9); assert!((fixed.sec_at(b.beat)-b.source).abs()<1e-9);
        }
    }
    #[test]
    fn rejects_reverse_close_edited_ratio_and_extreme_anchors_without_mutation() {
        let mut t=track(); let a=Anchor{beat:8.0,source:4.25}; let b=Anchor{beat:40.0,source:20.25};
        assert!(anchored(&t,b,a).is_err()); assert!(anchored(&t,a,Anchor{beat:9.0,source:4.75}).is_err());
        assert!(anchored(&t,a,Anchor{beat:40.0,source:39.0}).is_err());
        assert!(anchored(&t,a,Anchor{beat:f64::NAN,source:20.25}).is_err());
        t.sections=vec![(0.0,8.0),(0.0,8.0)]; assert!(anchored(&t,a,b).is_err());
        t.sections.clear(); t.beat_mult=2.0; assert!(anchored(&t,a,b).is_err());
    }
}
