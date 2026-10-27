//! Audible-quality regressions from the 2026-10-27 audit: treble at mismatched rates and clicks.
use super::*;
fn track(bpm:f64,dur:f64,lane:usize,stretch:u8)->crate::model::Track {
    let mut t:crate::model::Track=serde_json::from_value(serde_json::json!({"path":format!("t{lane}"),"title":"t","artist":"a","bpm":bpm,"offset":0.0,"duration":dur,"key":"8A","start_beat":0.0,"lane":lane,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}})).unwrap();
    t.stretch=stretch; t
}
fn sine(hz:f32,amp:f32,sr:u32,secs:f32)->Arc<Audio> { let v:Vec<f32>=(0..(sr as f32*secs) as usize).map(|i|amp*(std::f32::consts::TAU*hz*i as f32/sr as f32).sin()).collect(); Arc::new(Audio{sr,l:v.clone(),r:v}) }
fn max_step(v:&[f32])->f32 { v.windows(2).map(|w|(w[1]-w[0]).abs()).fold(0.0,f32::max) }
fn rms(l:&[f32])->f32 { let m=&l[l.len()/4..l.len()*3/4]; (m.iter().map(|x|x*x).sum::<f32>()/m.len() as f32).sqrt() }

#[test]
fn key_lock_keeps_treble_when_the_device_rate_differs() {
    for hz in [5000.0f32,10000.0,14000.0] {
        let level=|src:u32,dev:u32|{ let mut m=Mix { bpm:126.0, key_lock:true, ..Default::default() }; m.decks[0].vol=1.0; m.tracks.push(track(120.0,10.0,0,1));
            rms(&render_offline(Arc::new(Snap::build(&m,&[Some(sine(hz,0.3,src,10.0))])),dev,4.0,16.0).0) };
        let db=20.0*(level(44100,48000)/level(44100,44100)).log10();
        assert!(db.abs()<0.5,"{hz} Hz on a 48 kHz device: {db:+.2} dB");
    }
}

#[test]
fn deck_filter_re_engages_without_a_click() {
    let sr=44100; let mut m=Mix { bpm:120.0, ..Default::default() }; m.decks[0].vol=1.0;
    let mut t=track(120.0,20.0,0,2); while t.auto.len()<5 {t.auto.push(vec![]);}
    t.auto[4]=vec![(0.0,-0.3),(4.0,0.0),(12.0,0.0),(20.0,-0.3)]; m.tracks.push(t);
    let (l,_)=render_offline(Arc::new(Snap::build(&m,&[Some(sine(440.0,0.3,sr,20.0))])),sr,0.0,24.0);
    let bound=0.3*std::f32::consts::TAU*440.0/sr as f32;
    let step=max_step(&l[sr as usize/10..]); assert!(step<bound*1.5,"filter re-entry step {step} vs sine slope {bound}");
}

#[test]
fn mute_and_fader_changes_ramp_instead_of_stepping() {
    let sr=44100; let mut m=Mix { bpm:120.0, ..Default::default() }; m.decks[0].vol=0.85; m.tracks.push(track(120.0,20.0,0,2));
    let au=sine(1000.0,0.5,sr,20.0); let bound=0.5*0.85*std::f32::consts::TAU*1000.0/sr as f32;
    let mut muted=m.clone(); muted.decks[0].mute=true; let mut quiet=m.clone(); quiet.decks[0].vol=0.1; quiet.master=0.5;
    let mut b=SnapshotBuilder::default(); let mut e=Engine::new(sr as f32,Arc::new(Shared::new()));
    e.handle(Cmd::Prepared(b.prepare(Arc::new(Snap::build(&m,&[Some(au.clone())])),sr as f32))); e.handle(Cmd::Play(true));
    let mut out=vec![0.0;2*1011]; e.render(&mut out,2);
    for next in [&muted,&m,&quiet] {
        e.handle(Cmd::Prepared(b.prepare(Arc::new(Snap::build(next,&[Some(au.clone())])),sr as f32)));
        let mut after=vec![0.0;2*1024]; e.render(&mut after,2);
        let l:Vec<f32>=out.chunks(2).map(|c|c[0]).chain(after.chunks(2).map(|c|c[0])).collect();
        let step=max_step(&l[l.len()-1100..]); assert!(step<bound*1.3,"gain change stepped {step} (sine slope {bound})");
        out=after;
    }
}

#[test]
fn metronome_clicks_on_beats_after_seeks_and_loop_wraps() {
    let sr=44100.0; let run=|seek:f64,lp:Option<(f64,f64)>,secs:f32|{
        let mut s=Snap::build(&Mix::default(),&[]); s.bpm=120.0; s.metro=Some(1.0);
        let mut e=Engine::new(sr,Arc::new(Shared::new())); e.handle(Cmd::Snap(Arc::new(s))); e.handle(Cmd::Seek(seek)); e.handle(Cmd::Loop(lp)); e.handle(Cmd::Play(true));
        let mut out=vec![0.0;2*(sr*secs) as usize]; e.render(&mut out,2); out.chunks(2).map(|c|c[0].abs()).collect::<Vec<f32>>() };
    let peak=|v:&[f32],from:f32,to:f32|v[(from*sr) as usize..(to*sr) as usize].iter().fold(0.0f32,|a,b|a.max(*b));
    let mid=run(0.5,None,0.4); assert!(peak(&mid,0.0,0.05)<0.01,"no click mid-beat after a seek"); assert!(peak(&mid,0.25,0.3)>0.1,"the next beat clicks");
    let on=run(4.0,None,0.05); assert!(peak(&on,0.0,0.05)>0.1,"seeking onto a beat clicks");
    let looped=run(4.0,Some((4.0,5.0)),1.2); assert!(peak(&looped,0.5,0.55)>0.1 && peak(&looped,1.0,1.05)>0.1,"every loop pass clicks");
}

#[test]
fn loop_wraps_seeks_and_resumes_do_not_click() {
    let sr=44100; let mut m=Mix { bpm:120.0, ..Default::default() }; m.decks[0].vol=1.0; m.tracks.push(track(120.0,20.0,0,2));
    let snap=Arc::new(Snap::build(&m,&[Some(sine(101.3,0.5,sr,20.0))]));
    let bound=0.5*std::f32::consts::TAU*101.3/sr as f32;
    let mut e=Engine::new(sr as f32,Arc::new(Shared::new())); e.handle(Cmd::Snap(snap)); e.handle(Cmd::Seek(4.25)); e.handle(Cmd::Loop(Some((4.0,5.0)))); e.handle(Cmd::Play(true));
    let mut out=vec![0.0;2*sr as usize*2]; e.render(&mut out,2);
    let l:Vec<f32>=std::iter::once(0.0).chain(out.chunks(2).map(|c|c[0])).collect();
    let step=max_step(&l); assert!(step<bound*4.0,"loop wrap or start stepped {step} (sine slope {bound}; was 0.79 before the declick)");
    let quiet=l[(0.875*sr as f32) as usize-20..(0.875*sr as f32) as usize+20].iter().fold(0.0f32,|a,b|a.max(b.abs()));
    assert!(quiet<0.2,"the wrap point should dip, peak {quiet}");
}
