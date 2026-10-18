use super::*;
fn fixture(gain:f32,output:serde_json::Value,amplitude:f32,stretch:u8)->Arc<Snap> {
    let t:crate::model::Track=serde_json::from_value(serde_json::json!({"path":"levels.wav","title":"Levels","artist":"Test","bpm":120.0,"offset":0.0,"duration":4.0,"key":"8A","start_beat":0.0,"lane":0,"gain_db":gain,"stretch":stretch,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}})).unwrap();
    let mut json=serde_json::to_value(Mix::default()).unwrap();json["output"]=output;
    let mut mix:Mix=serde_json::from_value(json).unwrap();mix.bpm=120.0;mix.tracks.push(t);mix.decks[0].vol=1.0;
    let samples:Vec<_>=(0..32000).map(|i|amplitude*(std::f32::consts::TAU*220.0*i as f32/8000.0).sin()).collect();
    Arc::new(Snap::build(&mix,&[Some(Arc::new(Audio{sr:8000,l:samples.clone(),r:samples}))]))
}
#[test]
fn song_gain_is_applied_once_in_every_stretch_mode() {
    for (mode,bpm,tiny_loop) in [(1,120.0,false),(1,126.0,false),(1,126.0,true),(2,126.0,false),(3,126.0,false)] {
        let out=serde_json::json!({"enabled":true,"ceiling_db":0.0});
        let render=|gain,output| {
            let mut snap=fixture(gain,output,0.1,mode);Arc::get_mut(&mut snap).unwrap().bpm=bpm;
            let shared=Arc::new(Shared::new());let mut engine=Engine::new(8000.0,shared);
            engine.handle(Cmd::Snap(snap));if tiny_loop {engine.handle(Cmd::Loop(Some((0.0,0.015))));}
            engine.handle(Cmd::Play(true));let mut audio=vec![0.0;16000];engine.render(&mut audio,2);audio
        };
        let base=render(0.0,out.clone());let quiet=render(-6.0206,out);
        let error=base.iter().zip(&quiet).map(|(a,b)|(a*0.5-b).abs()).fold(0.0_f32,f32::max);
        assert!(base.iter().any(|v|v.abs()>0.02));assert!(error<1e-6,"mode {mode}: song gain was missing or applied twice: {error}");
    }
}
#[test]
fn output_bypass_and_sample_ceiling_change_real_audio() {
    let bypass=fixture(0.0,serde_json::json!({"enabled":false,"ceiling_db":-6.0}),1.5,2);
    let bypass=render_offline(bypass,8000,0.0,4.0).0;assert!(bypass.iter().any(|v|v.abs()>1.4),"bypass still applied the old soft clipper");
    let limited=fixture(0.0,serde_json::json!({"enabled":true,"ceiling_db":-6.0}),1.5,2);
    let limited=render_offline(limited,8000,0.0,4.0).0;
    let peak=limited.iter().map(|v|v.abs()).fold(0.0_f32,f32::max);assert!(peak>0.45 && peak<=0.501188,"ceiling was not honoured: {peak}");
}
#[test]
fn live_output_meter_includes_the_metronome() {
    let shared=Arc::new(Shared::new());let mut engine=Engine::new(8000.0,shared.clone());let mut mix=Mix::default();mix.metronome=true;mix.metro_vol=1.0;
    engine.handle(Cmd::Snap(Arc::new(Snap::build(&mix,&[]))));engine.handle(Cmd::Play(true));let mut audio=[0.0;128];engine.render(&mut audio,2);
    let peak=audio.iter().map(|v|v.abs()).fold(0.0_f32,f32::max);assert!(peak>0.1);
    assert!((Shared::get(&shared.meter[0])-peak).abs()<1e-6,"live meter omitted the audible click");
}
