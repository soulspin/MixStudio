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

#[test]
fn curved_clip_fades_change_the_envelope_and_roundtrip() {
    let mut snap=fixture(0.0,serde_json::json!({"enabled":false,"ceiling_db":0.0}),0.1,2);
    // Old projects remain linear. New shapes are independently stored for each edge.
    let mut value=serde_json::to_value(Mix::default()).unwrap();
    value["tracks"]=serde_json::json!([{"path":"curve.wav","title":"Curve","artist":"Test","bpm":120.0,"offset":0.0,"duration":4.0,"key":"8A","start_beat":0.0,"lane":2,"fade_in":2.0,"fade_out":2.0,"fade_curve":[0.5,-0.5],"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}]);
    let mix:Mix=serde_json::from_value(value).unwrap();
    let audio=snap.tracks[0].audio.clone();
    snap=Arc::new(Snap::build(&mix,&[Some(audio)]));
    assert!((snap.tracks[0].edge(1.0)-0.25).abs()<1e-6,"fade-in shape was ignored");
    assert!((snap.tracks[0].edge(7.0)-0.5_f32.sqrt()).abs()<1e-6,"fade-out shape was ignored");
    let saved=serde_json::to_value(&mix).unwrap();
    assert_eq!(saved["tracks"][0]["fade_curve"],serde_json::json!([0.5,-0.5]));
}
#[test]
fn stem_volume_curves_control_real_audio_in_all_stretch_modes() {
    for (mode,bpm) in [(1,120.0),(1,126.0),(2,126.0),(3,126.0)] {
        let mut mix=Mix::default();mix.bpm=bpm;mix.decks[0].vol=1.0;mix.output.enabled=false;
        let base=fixture(0.0,serde_json::json!({"enabled":false,"ceiling_db":0.0}),0.1,mode);
        let audio=base.tracks[0].audio.clone();
        let mut t:crate::model::Track=serde_json::from_value(serde_json::json!({"path":"voice.wav","title":"Voice","artist":"Test","bpm":120.0,"offset":0.0,"duration":4.0,"key":"8A","start_beat":0.0,"lane":0,"stretch":mode,"auto":[[],[],[],[],[],[],[],[],[[0.0,0.0],[8.0,0.0]]],"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}})).unwrap();
        let voice:Vec<i16>=audio.l.iter().map(|v|(v*32767.0).round() as i16).collect();let zeros=vec![0;voice.len()];
        let stems=Arc::new(Stems{d:[zeros.clone(),zeros.clone()],b:[zeros.clone(),zeros],v:[voice.clone(),voice],peaks:vec![]});
        mix.tracks.push(t.clone());
        let quiet=render_offline(Arc::new(Snap::build_with(&mix,&[Some(audio.clone())],&[Some(stems.clone())])),8000,0.0,6.0).0;
        t.auto.clear();mix.tracks[0]=t;
        let normal=render_offline(Arc::new(Snap::build_with(&mix,&[Some(audio)],&[Some(stems)])),8000,0.0,6.0).0;
        let peak=|v:&Vec<f32>|v.iter().map(|v|v.abs()).fold(0.0_f32,f32::max);
        assert!(peak(&normal)>0.02);assert!(peak(&quiet)<0.0002,"mode {mode}, {bpm} BPM: voice curve was ignored ({})",peak(&quiet));
    }
}
#[test]
fn repeated_source_uses_arranged_stem_curves_and_matches_live_export() {
    for mode in [1,2,3] {
        let mut mix=Mix::default();mix.bpm=126.0;mix.decks[0].vol=1.0;mix.output.enabled=false;
        let base=fixture(0.0,serde_json::json!({"enabled":false,"ceiling_db":0.0}),0.1,mode);let audio=base.tracks[0].audio.clone();
        let t=serde_json::from_value(serde_json::json!({"path":"repeat.wav","title":"Repeat","artist":"Test","bpm":120.0,"offset":0.0,"duration":4.0,"key":"8A","start_beat":0.0,"lane":0,"stretch":mode,"sections":[[0.0,2.0],[0.0,2.0]],"fade_in":0.25,"fade_out":0.25,"fade_curve":[0.5,-0.5],"auto":[[],[],[],[],[],[],[],[],[[0.0,1.0],[3.5,1.0],[4.0,0.0],[8.0,0.0]]],"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}})).unwrap();mix.tracks.push(t);
        let voice:Vec<i16>=audio.l.iter().map(|v|(v*32767.0).round() as i16).collect();let z=vec![0;voice.len()];
        let stems=Arc::new(Stems{d:[z.clone(),z.clone()],b:[z.clone(),z],v:[voice.clone(),voice],peaks:vec![]});
        let snap=Arc::new(Snap::build_with(&mix,&[Some(audio)],&[Some(stems)]));
        let (offline,_)=render_offline(snap.clone(),8000,0.0,8.0);
        let rms=|a:f64,b:f64|{let lo=(snap.secs(0.0,a)*8000.0) as usize;let hi=(snap.secs(0.0,b)*8000.0) as usize;(offline[lo..hi].iter().map(|v|v*v).sum::<f32>()/(hi-lo) as f32).sqrt()};
        assert!(rms(1.0,2.0)>0.02);assert!(rms(5.5,6.5)<0.0002,"mode {mode}: repeated source used first occurrence's stem envelope");
        let mut engine=Engine::new(8000.0,Arc::new(Shared::new()));engine.handle(Cmd::Snap(snap));engine.handle(Cmd::Seek(0.0));engine.handle(Cmd::Play(true));
        let mut live=Vec::new();while live.len()<offline.len(){let mut out=[0.0;512];engine.render(&mut out,2);live.extend(out.chunks(2).map(|s|s[0]));}
        assert!(offline.iter().zip(live).all(|(a,b)|(a-b).abs()<1e-6),"mode {mode}: live and export differed");
    }
}
#[test]
fn short_loop_refreshes_cached_audio_after_a_stem_curve_edit() {
    let base=fixture(0.0,serde_json::json!({"enabled":false,"ceiling_db":0.0}),0.1,1);let audio=base.tracks[0].audio.clone();
    let mut mix=Mix::default();mix.bpm=126.0;mix.decks[0].vol=1.0;mix.output.enabled=false;
    let t=serde_json::from_value(serde_json::json!({"path":"loop.wav","title":"Loop","artist":"Test","bpm":120.0,"offset":0.0,"duration":4.0,"key":"8A","start_beat":0.0,"lane":0,"stretch":1,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}})).unwrap();mix.tracks.push(t);
    let voice:Vec<i16>=audio.l.iter().map(|v|(v*32767.0).round() as i16).collect();let z=vec![0;voice.len()];let stems=Arc::new(Stems{d:[z.clone(),z.clone()],b:[z.clone(),z],v:[voice.clone(),voice],peaks:vec![]});
    let snapshot=|mix:&Mix|Arc::new(Snap::build_with(mix,&[Some(audio.clone())],&[Some(stems.clone())]));
    let mut engine=Engine::new(8000.0,Arc::new(Shared::new()));engine.handle(Cmd::Snap(snapshot(&mix)));engine.handle(Cmd::Loop(Some((1.0,1.015))));engine.handle(Cmd::Seek(1.0));engine.handle(Cmd::Play(true));
    let mut out=[0.0;4096];engine.render(&mut out,2);assert!(out.iter().any(|v|v.abs()>0.02));
    mix.tracks[0].auto_set(8,0.0,0.0);engine.handle(Cmd::Snap(snapshot(&mix)));engine.render(&mut out,2);
    assert!(out.iter().all(|v|v.abs()<0.0002),"short-loop cache ignored the edited stem envelope");
}
