//! Semantic checks at project/recovery boundaries, before UI or DSP activation.
use crate::{beatmap::BeatMap, model::Mix};

fn require(ok: bool, field: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(format!("Invalid project: {field}")) }
}
fn number(x: f64) -> bool { x.is_finite() && x.abs() <= 1e9 }
fn tempo(x: f64) -> bool { x.is_finite() && (1.0..=10000.0).contains(&x) }
fn level(x: f32, lo: f32, hi: f32) -> bool { x.is_finite() && (lo..=hi).contains(&x) }
fn map(m: &BeatMap, field: &str) -> Result<(), String> {
    require(tempo(m.bpm) && m.beats.len() >= 2 && m.beats.iter().all(|&x| number(x))
        && m.beats.windows(2).all(|w| (crate::beatmap::MIN_BEAT_GAP*0.5..=60.0).contains(&(w[1]-w[0]))), field)
}

/// Accept legacy duplicate automation times (dragging them is guarded), but
/// reject non-finite values and corrupt ordering before any indexing/interpolation.
pub fn validate_mix(m: &Mix) -> Result<(), String> {
    require(tempo(m.bpm), "mix BPM must be positive and finite")?;
    require(m.tempo_mode <= 3 && m.trans_stem_style <= 2, "mix mode")?;
    require(level(m.master,0.0,4.0) && level(m.metro_vol,0.0,1.0), "master/metronome level")?;
    require(level(m.output.ceiling_db,-12.0,0.0), "soft clipper ceiling")?;
    for d in &m.decks {
        require(level(d.vol,0.0,4.0) && [d.hi,d.mid,d.low,d.filter].iter().all(|&x| level(x,-1.0,1.0)), "deck level/EQ")?;
    }
    require(m.tempo_pts.iter().all(|&(b,t)| number(b) && tempo(t)), "tempo nodes")?;
    require([m.ramp.0,m.ramp.1].iter().all(|&t| t==0.0 || tempo(t)), "tempo ramp")?;
    for (i,t) in m.tracks.iter().enumerate() {
        let field=|name:&str|format!("track {} {name}",i+1);
        require(level(t.gain_db,-24.0,12.0), &field("gain"))?;
        require(t.lane < 4, &field("lane must be 0–3"))?;
        require(tempo(t.bpm) && number(t.beat_mult) && t.beat_mult>=1e-4 && t.beat_mult<=10000.0, &field("tempo/beat ratio"))?;
        require(number(t.duration) && t.duration>0.0 && number(t.offset) && number(t.start_beat), &field("duration/placement"))?;
        require([t.trim_in,t.trim_out,t.fade_in,t.fade_out].iter().all(|&x| number(x) && x>=0.0)
            && t.trim_in+t.trim_out<=t.duration+1e-7, &field("trim/fades"))?;
        require(t.grid_mode<=4 && t.stretch<=3 && t.stem_view<=2 && (t.trans.ratio as usize)<crate::model::RATIOS.len(), &field("mode"))?;
        for grid in [&t.map,&t.flex,&t.imported].into_iter().flatten() { map(grid,&field("beat map must be strictly increasing"))?; }
        require(t.sections.iter().all(|&(a,b)| number(a) && number(b) && a>=0.0 && b>a && b<=t.duration+0.025), &field("source sections"))?;
        require(t.stem_mutes.iter().all(|p| p.stem<4 && number(p.a) && number(p.b) && p.b>=p.a), &field("stem ranges"))?;
        require(t.auto.len()<=9, &field("automation parameter"))?;
        require(t.fade_curve.iter().all(|&v|level(v,-1.0,1.0)), &field("fade shape"))?;
        for (k,p) in t.auto.iter().enumerate() {
            require(p.iter().all(|&(b,v)| number(b) && if k>=5 {level(v,0.0,1.0)}else{level(v,-16.0,16.0)}) && p.windows(2).all(|w| w[1].0>=w[0].0), &field("automation points"))?;
        }
        for p in [&t.trans.curve_out,&t.trans.curve_in] {
            require(p.iter().all(|&(b,v)| level(b,0.0,1.0) && level(v,0.0,4.0)) && p.windows(2).all(|w| w[1].0>=w[0].0), &field("transition curve"))?;
        }
    }
    for f in &m.fx {
        require(f.target<=4 && (f.kind!=crate::fx::FxKind::Scratch || f.target<4), "effect target")?;
        require(number(f.start) && number(f.len) && f.len>=0.0 && number(f.tail) && f.tail>=0.0 && number(f.end()), "effect range")?;
        require([f.mix,f.decay,f.depth,f.ease].iter().all(|&x| level(x,0.0,1.0)) && level(f.time,0.0,10000.0), "effect parameters")?;
    }
    for p in m.phrase_edit.values() {
        require(p.iter().all(|&(b,k)| number(b) && k<5) && p.windows(2).all(|w|w[1].0>=w[0].0), "phrase labels")?;
    }
    require(m.cue_marks.values().all(|v|v.iter().all(|(s,_)|number(*s))), "cue times")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Mix {
        serde_json::from_str(include_str!("../tests/fixtures/unstable-flex-pair.json")).unwrap()
    }
    #[test]
    fn existing_projects_and_legacy_duplicate_automation_are_accepted() {
        let mut m=fixture(); validate_mix(&m).unwrap();
        assert!(m.tracks.iter().all(|t|t.gain_db==0.0));assert_eq!(m.output,crate::levels::OutputSettings::default());
        m.tracks[0].auto=vec![vec![(8.0,0.1),(8.0,0.5),(8.0,0.9)]];
        validate_mix(&m).unwrap(); validate_mix(&Mix::default()).unwrap();
    }
    #[test]
    fn corrupt_numeric_modes_maps_and_ranges_are_rejected() {
        let original=fixture();
        for case in 0..16 {
            let mut m=original.clone();
            match case {
                0=>m.tracks[0].lane=4,
                1=>m.bpm=0.0,
                2=>m.tracks[0].bpm=f64::NAN,
                3=>m.tracks[0].beat_mult=0.0,
                4=>m.tracks[0].map=Some(std::sync::Arc::new(BeatMap {bpm:120.0,beats:vec![]})),
                5=>m.tracks[0].map=Some(std::sync::Arc::new(BeatMap {bpm:120.0,beats:vec![0.0,1.0,0.5]})),
                6=>m.tracks[0].sections=vec![(1.0,0.5)],
                7=>m.tracks[0].stretch=255,
                8=>m.tempo_pts=vec![(0.0,-120.0)],
                9=>m.fx.push(crate::fx::FxBlock::new(crate::fx::FxKind::Echo,5,0.0)),
                10=>m.tracks[0].map=Some(std::sync::Arc::new(BeatMap {bpm:120.0,beats:(0..16).map(|i|i as f64*0.0001).collect()})),
                12=>m.tracks[0].gain_db=f32::NAN,
                13=>m.tracks[0].gain_db=13.0,
                14=>m.output.ceiling_db=f32::INFINITY,
                15=>m.output.ceiling_db=1.0,
                _=>m.tracks[0].auto=vec![vec![(1.0,f32::INFINITY)]],
            }
            assert!(validate_mix(&m).is_err(),"accepted corrupt case {case}");
        }
    }
}
