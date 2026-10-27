#[test]
fn clip_curve_handle_drags_and_undoes_in_one_step() {
    let (mut a,ctx,_rx,dir)=setup();a.mix.tracks[0].lane=2;a.mix.tracks[0].fade_in=8.0;
    frame(&mut a,&ctx,vec![],1000.0);let output=frame(&mut a,&ctx,vec![],1000.0);
    let target=output.shapes.iter().find_map(|s|if let egui::epaint::Shape::Circle(c)=&s.shape {if c.radius==3.5 && c.center.y>400.0 {Some(c.center)}else{None}}else{None}).expect("fade midpoint handle must be visible");
    let pointer=|p,pressed|egui::Event::PointerButton{pos:p,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(target),pointer(target,true)],1000.0);
    let end=target+vec2(0.0,10.0);frame(&mut a,&ctx,vec![egui::Event::PointerMoved(end)],1000.0);frame(&mut a,&ctx,vec![pointer(end,false)],1000.0);
    assert!(a.mix.tracks[0].fade_curve[0]>0.1,"midpoint did not bend the fade");assert_eq!(a.mix.tracks[0].start_beat,0.0);assert_eq!(a.undo.len(),1);
    a.do_undo();assert_eq!(a.mix.tracks[0].fade_curve,[0.0;2]);drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn stem_curve_click_edits_only_vocals_and_undoes() {
    let (mut a,ctx,_rx,dir)=setup();let n=a.store["test.wav"].0.l.len();let z=vec![0;n];
    a.stems.insert("test.wav".into(),Arc::new(crate::stems::Stems{d:[z.clone(),z.clone()],b:[z.clone(),z.clone()],v:[z.clone(),z],peaks:vec![[0;4];600]}));
    a.mix.tracks[0].stem_view=2;a.auto_track=Some(0);
    frame(&mut a,&ctx,vec![],1000.0);let output=frame(&mut a,&ctx,vec![],1000.0);
    let label=|name:&str|output.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape {if t.galley.text()==name {Some(t.pos)}else{None}}else{None}).expect(name);
    let height=label("Vocals").y-label("Melody").y;
    let target=pos2(HEAD_W+8.0+20.0*7.0,label("Vocals").y+10.0+(height-15.0)/2.0);
    let pointer=|pressed|egui::Event::PointerButton{pos:target,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(target),pointer(true)],1000.0);frame(&mut a,&ctx,vec![pointer(false)],1000.0);
    let t=&a.mix.tracks[0];assert_eq!(t.start_beat,0.0);assert_eq!(t.auto.len(),9,"stem row did not create its volume curve");
    assert!(t.auto[..8].iter().all(Vec::is_empty));assert!((t.auto[8][0].1-0.5).abs()<0.06);assert_eq!(a.undo.len(),1);
    crate::validation::validate_mix(&a.mix).unwrap();a.do_undo();assert!(a.mix.tracks[0].auto.is_empty());drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn sample_copy_is_atomic_undoable_and_retains_stem_isolation() {
    let (mut a,_ctx,_rx,dir)=setup();let original=serde_json::to_string(&a.mix).unwrap();
    a.finish_sample_copy(sample_ui::CopyDrag{ti:0,a:8.0,b:16.0,anchor:8.0,stems:Some(vec![3])},2,32.0);
    assert_eq!(a.mix.tracks.len(),2);assert_eq!(a.undo.len(),1);assert!(a.mix.tracks[1].muted(0,8.0));assert!(!a.mix.tracks[1].muted(3,8.0));
    let saved=serde_json::to_string(&a.mix).unwrap();crate::validation::validate_mix(&serde_json::from_str(&saved).unwrap()).unwrap();
    a.do_undo();assert_eq!(serde_json::to_string(&a.mix).unwrap(),original);a.do_redo();assert_eq!(serde_json::to_string(&a.mix).unwrap(),saved);
    let before=a.undo.len();a.finish_sample_copy(sample_ui::CopyDrag{ti:0,a:2.0,b:1.0,anchor:2.0,stems:None},2,32.0);assert_eq!(a.undo.len(),before);assert_eq!(a.mix.tracks.len(),2);
    drop(a);std::fs::remove_dir_all(dir).ok();
}

#[test]
fn reopening_and_relinking_request_stems_for_already_loaded_sources() {
    let (mut a,_ctx,_rx,dir)=setup();let (tx,jobs)=crossbeam_channel::unbounded();a.stem_tx=tx;
    let mut mix=a.mix.clone();mix.tracks[0].auto_set(8,0.0,0.5);a.persistence.enabled=false;
    a.request_project(persistence::Target{mix,file:None,clean:true,restored:None,original:None});
    assert!(jobs.try_recv().is_ok(),"already-loaded sources skipped required stem preparation");a.stem_prog.clear();
    let old=a.mix.tracks[0].path.clone();let (audio,analysis)=a.store[&old].clone();
    a.persistence.candidate=Some(persistence::Candidate{old,path:dir.join("relocated.wav"),audio,analysis,confirmed:true});
    a.apply_relink();assert!(jobs.try_recv().is_ok(),"relinked sources skipped required stem preparation");drop(a);std::fs::remove_dir_all(dir).ok();
}
fn editing_frame(a:&mut App,ctx:&egui::Context,events:Vec<egui::Event>,modifiers:egui::Modifiers)->egui::FullOutput {
    ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(1000.0,800.0))),events,modifiers,..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|a.timeline(ui));})
}
#[test]
fn alt_drag_selected_section_copies_to_the_drop_position() {
    let (mut a,ctx,_rx,dir)=setup();a.section_range=Some((0,8.0,16.0));a.remember_section();a.select_sections=true;
    frame(&mut a,&ctx,vec![],1000.0);let output=frame(&mut a,&ctx,vec![],1000.0);
    let row=output.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape {if t.galley.text()=="›  Sample 1"{Some(t.pos.y+5.0)}else{None}}else{None}).unwrap();
    let bx=|b:f32|HEAD_W+8.0+(b+4.0)*7.0;let start=pos2(bx(12.0),150.0);let end=pos2(bx(36.0),row);
    let mods=egui::Modifiers{alt:true,..Default::default()};let pointer=|p,pressed|egui::Event::PointerButton{pos:p,button:egui::PointerButton::Primary,pressed,modifiers:mods};
    editing_frame(&mut a,&ctx,vec![egui::Event::PointerMoved(start),pointer(start,true)],mods);
    editing_frame(&mut a,&ctx,vec![egui::Event::PointerMoved(end)],mods);
    editing_frame(&mut a,&ctx,vec![pointer(end,false)],mods);
    assert_eq!(a.mix.tracks.len(),2,"Alt-drag did not create a sample");assert_eq!(a.undo.len(),1);assert_eq!(a.mix.tracks[1].lane,2);
    assert!((a.mix.tracks[1].vis_start(0.0)-32.0).abs()<1e-8,"drop did not retain the grab offset");assert!((a.mix.tracks[1].end(0.0)-40.0).abs()<1e-8);
    a.do_undo();assert_eq!(a.mix.tracks.len(),1);drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn sample_rectangle_selects_both_lanes_and_drag_keeps_spacing() {
    let (mut a,ctx,_rx,dir)=setup();let first=a.mix.sample_range(0,8.0,16.0,2,8.0,None).unwrap();let second=a.mix.sample_range(0,8.0,16.0,3,24.0,None).unwrap();
    frame(&mut a,&ctx,vec![],1000.0);let output=frame(&mut a,&ctx,vec![],1000.0);
    let row=|name:&str|output.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape {if t.galley.text()==name{Some(t.pos.y+5.0)}else{None}}else{None}).unwrap();
    let bx=|b:f32|HEAD_W+8.0+(b+4.0)*7.0;let start=pos2(bx(40.0),row("›  Sample 1"));let end=pos2(bx(6.0),row("›  Sample 2"));
    let shift=egui::Modifiers{shift:true,..Default::default()};let pointer=|p,pressed,modifiers|egui::Event::PointerButton{pos:p,button:egui::PointerButton::Primary,pressed,modifiers};
    editing_frame(&mut a,&ctx,vec![egui::Event::PointerMoved(start),pointer(start,true,shift)],shift);
    editing_frame(&mut a,&ctx,vec![egui::Event::PointerMoved(end)],shift);editing_frame(&mut a,&ctx,vec![pointer(end,false,shift)],shift);
    assert_eq!(a.samples.selected.iter().copied().collect::<Vec<_>>(),vec![first,second]);
    let start=pos2(bx(12.0),row("›  Sample 1"));let end=start+vec2(56.0,0.0);
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(start),pointer(start,true,Default::default())],1000.0);
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(end)],1000.0);frame(&mut a,&ctx,vec![pointer(end,false,Default::default())],1000.0);
    assert!((a.mix.tracks[first].vis_start(0.0)-16.0).abs()<1e-8);assert!((a.mix.tracks[second].vis_start(0.0)-32.0).abs()<1e-8);assert_eq!(a.undo.len(),1);
    a.do_undo();assert_eq!(a.mix.tracks[first].vis_start(0.0),8.0);assert_eq!(a.mix.tracks[second].vis_start(0.0),24.0);drop(a);std::fs::remove_dir_all(dir).ok();
}

#[test]
fn fades_continue_across_splits_stay_inside_after_section_delete_and_dragging_cannot_panic() {
    let (mut a,ctx,_rx,dir)=setup(); a.mix.tracks[0].fade_in=48.0; a.mix.tracks[0].fade_out=40.0;
    let right=a.mix.split(0,16.0).unwrap();
    // The halves continue the original curves (neutral split); the right half starts 16 beats into the fade-in.
    assert_eq!(a.mix.tracks[right].fade_skip[0],16.0); assert_eq!(a.mix.tracks[right].fade_in,48.0);
    a.tl_drag=Some(TlDrag::Fade{ti:0,left:false});
    let p=pos2(300.0,150.0);
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(p),egui::Event::PointerButton{pos:p,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}],1000.0);
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(p+vec2(5.0,0.0))],1000.0);
    let t=&a.mix.tracks[0]; assert!(t.fade_out>=0.0 && t.fade_out-t.fade_skip[1]<=t.end(0.0)-t.vis_start(0.0)+1e-9);
    let t=&mut a.mix.tracks[right]; t.fade_in=0.0; t.fade_out=40.0; let (vs,ve)=(t.vis_start(0.0),t.end(0.0));
    a.mix.delete_section(right,vs+4.0,ve-4.0).unwrap();
    let t=&a.mix.tracks[right]; assert!(t.fade_out<=t.end(0.0)-t.vis_start(0.0)+1e-9,"section delete left a fade longer than the clip");
    drop(a);std::fs::remove_dir_all(dir).ok();
}

#[test]
fn delete_or_undo_during_an_edge_drag_cannot_overwrite_another_track() {
    for undo in [false,true] {
        let (mut a,ctx,_rx,dir)=setup();
        let mut b=a.mix.tracks[0].clone();b.path="other.wav".into();b.lane=1;b.start_beat=48.0;b.title="Other".into();
        let (au,an)=a.store["test.wav"].clone();a.store.insert("other.wav".into(),(au,an));a.mix.tracks.push(b);
        frame(&mut a,&ctx,vec![],1000.0);
        let pos=pos2(600.0,40.0);
        frame(&mut a,&ctx,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}],1000.0);
        a.snapshot();a.tl_drag=Some(TlDrag::Edge{ti:0,left:false,original:Box::new(a.mix.tracks[0].clone())});a.sel=Some(0);a.auto_track=Some(1);
        if undo { a.mix.tracks.remove(0); a.do_undo(); a.mix.tracks.swap(0,1); } else { a.remove_sel(); }
        let before:Vec<_>=a.mix.tracks.iter().map(|t|(t.path.clone(),t.start_beat,t.trim_out)).collect();
        frame(&mut a,&ctx,vec![egui::Event::PointerMoved(pos2(620.0,40.0))],1000.0);
        let after:Vec<_>=a.mix.tracks.iter().map(|t|(t.path.clone(),t.start_beat,t.trim_out)).collect();
        assert_eq!(before,after,"a stale drag rewrote tracks after {}",if undo {"undo"} else {"delete"});
        if !undo { assert_eq!(a.auto_track,Some(0),"the automation lane must follow its track"); }
        drop(a);std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn grid_edits_on_one_half_of_a_split_song_keep_the_song_together() {
    let (mut a,_ctx,_rx,dir)=setup();
    let mut b=a.mix.tracks[0].clone(); b.path="b.wav".into(); b.lane=1; b.start_beat=16.0; b.duration=64.0;
    let (au,an)=a.store["test.wav"].clone(); a.store.insert("b.wav".into(),(au,an)); a.mix.tracks.push(b);
    let right=a.mix.split(1,40.0).unwrap(); let before=a.mix.logical_transitions();
    a.nudge_incoming(1,1.0); a.nudge_incoming(1,-10.0);
    let gap=a.mix.tracks[right].vis_start(0.0)-a.mix.tracks[1].end(0.0); assert!(gap.abs()<1e-7,"halves drifted apart by {gap}");
    let after=a.mix.logical_transitions(); assert_eq!(after.len(),before.len());
    assert!((after[0].3-before[0].3).abs()<0.05,"transition end moved from {} to {}",before[0].3,after[0].3);
    a.mix.tracks.remove(0); a.mix.alternate_main_lanes(); assert_eq!(a.mix.tracks[0].lane,a.mix.tracks[1].lane,"still one song on one deck");
    drop(a);std::fs::remove_dir_all(dir).ok();
}
