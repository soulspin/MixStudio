#[test]
fn library_shows_actual_filenames_instead_of_only_tags() {
    let (mut a,ctx,_rx,dir)=setup();
    a.lib.cfg.folders=vec!["/Music".into()];a.lib.states.insert("/Music".into(),crate::library::FolderState::Available(2));
    a.lib.index.entries=vec![crate::library::Entry{path:"/Music/House/Artist - Long Extended Club Mix.flac".into(),folder:"/Music".into(),title:"Metadata title".into(),artist:"Metadata artist".into(),bpm:Some(126.0),key:Some("8A".into()),..Default::default()}];a.lib.gen+=1;
    let draw=|a:&mut App|ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(1000.0,500.0))),..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|a.library_tab(ui));});
    draw(&mut a);let out=draw(&mut a);
    assert!(out.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.text()=="Artist - Long Extended Club Mix.flac")),"filename was hidden behind metadata tags");
    drop(a);std::fs::remove_dir_all(dir).ok();
}
fn library_result(name:&str)->(PathBuf,Result<(Audio,Analysis,bool),String>) {
    (PathBuf::from(name),Ok((Audio{sr:8000,l:vec![0.0;256000],r:vec![0.0;256000]},Analysis{title:name.into(),bpm:120.0,offset:0.375,duration:32.0,peak_sr:8000,..Default::default()},false)))
}
#[test]
fn library_main_deck_drops_align_downbeats_despite_different_file_lead_ins() {
    let (mut a,_ctx,_rx,dir)=setup();let id=a.persistence.id.clone();
    let mut second=library_result("two.wav");second.1.as_mut().unwrap().1.offset=0.21;
    a.finish_library_batch(&id,Some(library_drop::Target{lane:0,beat:20.0}),vec![library_result("one.wav"),second]);
    a.mix.resolve_ratios();
    for t in &a.mix.tracks[1..] {
        assert!((t.start_beat/4.0-(t.start_beat/4.0).round()).abs()<1e-9,"main deck downbeat lost its bar alignment: {}",t.start_beat);
    }
    assert_eq!(a.mix.tracks[1].start_beat,20.0);
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_main_drop_at_zero_keeps_the_intro_and_unsnapped_targets_stay_fractional() {
    let (mut a,_ctx,_rx,dir)=setup();let id=a.persistence.id.clone();
    a.finish_library_batch(&id,Some(library_drop::Target{lane:1,beat:0.0}),vec![library_result("intro.wav")]);
    let t=a.mix.tracks.last().unwrap();assert_eq!(t.start_beat,4.0);assert!(t.vis_start(0.0)>=0.0);
    a.snapping=Snapping::Off;
    a.finish_library_batch(&id,Some(library_drop::Target{lane:0,beat:20.375}),vec![library_result("fractional.wav")]);
    assert_eq!(a.mix.tracks.last().unwrap().start_beat,20.375);
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_drop_keeps_the_downbeat_position_order_and_one_undo() {
    let (mut a,_ctx,_rx,dir)=setup();let before=serde_json::to_string(&a.mix).unwrap();a.loading=2;
    let id=a.persistence.id.clone();a.finish_library_batch(&id,Some(library_drop::Target{lane:1,beat:20.0}),vec![library_result("Z-first.wav"),library_result("A-second.wav")]);
    assert_eq!(a.loading,0);assert_eq!(a.undo.len(),1);assert_eq!(a.mix.tracks.len(),3);
    assert_eq!(a.mix.tracks[1].title,"Z-first.wav");assert_eq!(a.mix.tracks[2].title,"A-second.wav");assert_eq!(a.mix.tracks[1].lane,1);assert_eq!(a.mix.tracks[2].lane,0);
    assert_eq!(a.mix.tracks[1].start_beat,20.0);assert_eq!(a.mix.tracks[2].start_beat,52.0);
    a.do_undo();assert_eq!(serde_json::to_string(&a.mix).unwrap(),before);drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_import_failures_locks_and_stale_documents_do_not_create_undo_entries() {
    let (mut a,_ctx,_rx,dir)=setup();let before=serde_json::to_string(&a.mix).unwrap();let id=a.persistence.id.clone();
    a.loading=1;a.finish_library_batch(&id,None,vec![(PathBuf::from("missing.wav"),Err("missing".into()))]);assert_eq!(a.loading,0);assert!(a.undo.is_empty());
    a.lock=true;a.finish_library_batch(&id,None,vec![library_result("blocked.wav")]);a.lock=false;
    a.finish_library_batch("old-project",None,vec![library_result("stale.wav")]);
    assert!(a.undo.is_empty());assert_eq!(serde_json::to_string(&a.mix).unwrap(),before);drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_selection_is_added_in_visible_order_and_does_not_leak_through_filters() {
    let (mut a,_ctx,_rx,dir)=setup();a.lib.index.entries=vec![crate::library::Entry{path:"/Music/alpha.wav".into(),..Default::default()},crate::library::Entry{path:"/Music/zebra.wav".into(),..Default::default()}];a.lib.gen+=1;a.lib_sort=crate::library::SortCol::Filename;a.lib_asc=false;
    a.refresh_library_view();a.select_library_row(0,Default::default());a.select_library_row(1,egui::Modifiers{shift:true,..Default::default()});assert_eq!(a.library_selected_paths(),vec!["/Music/zebra.wav","/Music/alpha.wav"]);
    a.lib_q="alpha".into();a.refresh_library_view();assert_eq!(a.library_selected_paths(),vec!["/Music/alpha.wav"]);assert_eq!(a.lib_sel.len(),1);assert_eq!(a.lib_anchor,None);
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn dragging_empty_row_space_to_a_sample_lane_keeps_visible_selection_order() {
    let (mut a,ctx,_rx,dir)=setup();a.lib.cfg.folders=vec!["/Music".into()];
    a.lib.index.entries=vec![crate::library::Entry{path:"/Music/A.wav".into(),folder:"/Music".into(),..Default::default()},crate::library::Entry{path:"/Music/Z.wav".into(),folder:"/Music".into(),..Default::default()}];a.lib.gen+=1;a.lib_asc=false;
    let draw=|a:&mut App,events|ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(1100.0,900.0))),events,..Default::default()},|ctx|{
        egui::TopBottomPanel::bottom("test_library").exact_height(360.0).show(ctx,|ui|a.library_tab(ui));
        egui::CentralPanel::default().show(ctx,|ui|a.timeline(ui));a.library_drag_frame(ctx);
    });
    draw(&mut a,vec![]);draw(&mut a,vec![]);a.select_library_row(0,Default::default());a.select_library_row(1,egui::Modifiers{shift:true,..Default::default()});let out=draw(&mut a,vec![]);
    let text=|name:&str|out.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape {if t.galley.text()==name{Some(t.pos)}else{None}}else{None}).expect(name);
    let start=pos2(text("BPM").x-22.0,text("Z.wav").y+25.0);
    let g=a.library_ui.geometry.as_ref().unwrap();let row=g.rows.iter().find(|(l,_)|*l==2).unwrap().1;let end=pos2(g.left+(20.0-g.scroll) as f32*g.ppb,row.center().y);
    assert_eq!(a.library_drop_target(end),Some(library_drop::Target{lane:2,beat:20.0}));assert!(a.library_drop_target(pos2(g.left-10.0,row.center().y)).is_none());
    let pointer=|p,pressed|egui::Event::PointerButton{pos:p,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
    draw(&mut a,vec![egui::Event::PointerMoved(start),pointer(start,true)]);draw(&mut a,vec![egui::Event::PointerMoved(end)]);
    let payload=egui::DragAndDrop::payload::<LibDrag>(&ctx).expect("the whitespace in a file row must be draggable");assert_eq!(payload.0,vec!["/Music/Z.wav","/Music/A.wav"]);
    draw(&mut a,vec![pointer(end,false)]);
    match a.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(){Msg::LibraryBatch(_,target,rows)=>{assert_eq!(target,Some(library_drop::Target{lane:2,beat:20.0}));assert_eq!(rows[0].0,PathBuf::from("/Music/Z.wav"));assert_eq!(rows[1].0,PathBuf::from("/Music/A.wav"));},_=>panic!("drop did not use the ordered library batch")}
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn batch_append_resolves_bpm_ratios_before_placing_the_following_song() {
    let (mut a,_ctx,_rx,dir)=setup();let mut first=library_result("slow.wav");first.1.as_mut().unwrap().1.bpm=60.0;first.1.as_mut().unwrap().1.duration=120.0;first.1.as_mut().unwrap().0.l.resize(960000,0.0);first.1.as_mut().unwrap().0.r.resize(960000,0.0);
    let id=a.persistence.id.clone();a.finish_library_batch(&id,None,vec![first,library_result("next.wav")]);a.mix.resolve_ratios();
    assert_eq!(a.mix.tracks[1].beat_mult,2.0);
    assert!((a.mix.tracks[2].vis_start(0.0)-a.mix.tracks[1].end(0.0)+64.0).abs()<1e-8,"append used the unresolved source duration");
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn fractional_sample_drops_survive_failures_and_ratio_resolution() {
    let (mut a,_ctx,_rx,dir)=setup();a.snapping=Snapping::Off;let id=a.persistence.id.clone();
    a.finish_library_batch(&id,Some(library_drop::Target{lane:3,beat:20.375}),vec![library_result("one.wav"),(PathBuf::from("bad.wav"),Err("decode failed".into())),library_result("two.wav")]);
    a.mix.resolve_ratios();assert_eq!(a.mix.tracks.len(),3);assert_eq!(a.mix.tracks[1].lane,3);assert_eq!(a.mix.tracks[2].lane,3);
    assert!((a.mix.tracks[1].vis_start(0.0)-20.375).abs()<1e-8);assert!((a.mix.tracks[2].vis_start(0.0)-84.375).abs()<1e-8);assert!(a.status.contains("1 failed"));assert_eq!(a.undo.len(),1);
    drop(a);std::fs::remove_dir_all(dir).ok();
}

#[test]
fn library_analysis_refresh_keeps_shift_selection_anchored_to_the_same_file() {
    let (mut a,_ctx,_rx,dir)=setup();a.lib.index.entries=vec![crate::library::Entry{path:"/Music/A.wav".into(),..Default::default()},crate::library::Entry{path:"/Music/B.wav".into(),..Default::default()},crate::library::Entry{path:"/Music/C.wav".into(),..Default::default()}];a.lib.gen+=1;
    a.refresh_library_view();a.select_library_row(0,Default::default());a.lib.index.entries[1].bpm=Some(126.0);a.lib.gen+=1;a.refresh_library_view();
    a.select_library_row(2,egui::Modifiers{shift:true,..Default::default()});assert_eq!(a.library_selected_paths(),vec!["/Music/A.wav","/Music/B.wav","/Music/C.wav"]);
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn clicking_a_folder_filters_its_descendants_without_matching_similar_names() {
    let (mut a,ctx,_rx,dir)=setup();a.lib.cfg.folders=vec!["/Music".into()];
    a.lib.index.entries=vec![crate::library::Entry{path:"/Music/House/one.wav".into(),folder:"/Music".into(),..Default::default()},crate::library::Entry{path:"/Music/House/Deep/two.wav".into(),folder:"/Music".into(),..Default::default()},crate::library::Entry{path:"/Music/House2/three.wav".into(),folder:"/Music".into(),..Default::default()}];a.lib.gen+=1;
    let draw=|a:&mut App,events|ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(1000.0,420.0))),events,..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|a.library_tab(ui));});
    draw(&mut a,vec![]);let out=draw(&mut a,vec![]);
    let target=out.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape {if t.galley.text()=="House"{Some(t.pos+t.galley.size()*0.5)}else{None}}else{None}).unwrap();
    let pointer=|pressed|egui::Event::PointerButton{pos:target,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
    draw(&mut a,vec![egui::Event::PointerMoved(target),pointer(true)]);draw(&mut a,vec![pointer(false)]);
    assert_eq!(a.library_ui.folder.as_deref(),Some("/Music/House"));assert_eq!(a.lib_view.len(),2);assert!(a.lib_view.iter().all(|&i|!a.lib.index.entries[i].path.contains("House2")));
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_keys_select_all_visible_and_enter_adds_them_in_order() {
    let (mut a,ctx,_rx,dir)=setup();a.tab=Tab::Library;
    a.lib.index.entries=vec![crate::library::Entry{path:"/Music/b.wav".into(),..Default::default()},crate::library::Entry{path:"/Music/a.wav".into(),..Default::default()},crate::library::Entry{path:"/Music/skip.wav".into(),..Default::default()}];a.lib.gen+=1;
    a.lib_q=".wav".into();a.lib_sort=crate::library::SortCol::Filename;a.lib_asc=true;a.refresh_library_view();
    let key=|key,command|{let modifiers=egui::Modifiers{command,ctrl:command,..Default::default()};egui::RawInput{modifiers,events:vec![egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers}],..Default::default()}};
    let _=ctx.run(key(egui::Key::A,true),|ctx|a.library_keys(ctx));
    assert_eq!(a.library_selected_paths(),vec!["/Music/a.wav","/Music/b.wav","/Music/skip.wav"]);
    a.lib_q="a.wav".into();a.refresh_library_view();assert_eq!(a.library_selected_paths(),vec!["/Music/a.wav"]);
    let before=a.loading;let _=ctx.run(key(egui::Key::Enter,false),|ctx|a.library_keys(ctx));
    assert!(a.loading>before,"Enter should start adding the selection");
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_preview_plays_the_source_at_its_own_tempo_and_restores_the_mix() {
    let (mut a,ctx,_rx,dir)=setup();a.tab=Tab::Library;
    let mut seed=7u32;let noise:Vec<f32>=(0..8000*32).map(|_|{seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);((seed>>8) as f32/8388608.0-1.0)*0.4}).collect();
    let an=Analysis{bpm:123.0,offset:0.3,duration:32.0,peak_sr:8000,peaks:vec![[80,100,90];600],..Default::default()};
    a.store.insert("test.wav".into(),(Arc::new(Audio{sr:8000,l:noise.clone(),r:noise.clone()}),Arc::new(an)));
    a.seek(10.0);a.loop_on=true;a.set_loop(Some((8.0,16.0)));
    a.toggle_listen("test.wav".into(),"Test".into(),&ctx);
    assert!(a.listen.as_ref().is_some_and(|l|l.ready.is_some()));assert_eq!(a.beat(),10.0,"the mix playhead must stay put");assert_eq!(a.active_loop,None);
    let snap=a.build_playback_snap();assert_eq!(snap.tracks.len(),1);
    let beat=a.engine_beat();let sec=0.3+(beat-4.0)*60.0/123.0;assert!((sec-32.0/3.0).abs()<1e-6,"no drop detected: start a third of the way in");
    let (l,_)=crate::engine::render_offline(Arc::new(snap),8000,beat,beat+2.0);let s0=(sec*8000.0).round() as usize;let n=l.len().min(4000);
    let corr=|lag:i64|{let src=&noise[(s0 as i64+lag) as usize..];let (mut xy,mut xx,mut yy)=(0.0,0.0,0.0);for i in 200..n {let (x,y)=(l[i] as f64,src[i] as f64);xy+=x*y;xx+=x*x;yy+=y*y;}xy/(xx*yy).sqrt().max(1e-12)};
    assert!((-3..=3).map(corr).fold(f64::MIN,f64::max)>0.9,"preview is not the unstretched source at the cue");
    a.stop_listen();assert!(a.listen.is_none());assert_eq!(a.beat(),10.0);assert_eq!(a.active_loop,Some((8.0,16.0)));assert!(a.loop_on);
    a.toggle_listen("test.wav".into(),"Test".into(),&ctx);a.tab=Tab::Track;a.validate_listen();assert!(a.listen.is_none(),"leaving the Library stops the preview");
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn edits_and_timeline_clicks_during_a_preview_use_the_mix_position() {
    let (mut a,ctx,_rx,dir)=setup();a.tab=Tab::Library;a.mix.tracks[0].duration=32.0;
    a.seek(10.0);a.toggle_listen("test.wav".into(),"Test".into(),&ctx);
    assert!((a.engine_beat()-10.0).abs()>1.0,"the preview plays elsewhere in its own file");
    // Ctrl+E splits at the visible mix playhead, not at the preview position.
    a.tl_hover=None;a.split_here();
    assert!(a.mix.tracks.iter().any(|t|(t.end(0.0)-10.0).abs()<1e-6),"split at {:?}",a.mix.tracks.iter().map(|t|t.end(0.0)).collect::<Vec<_>>());
    a.do_undo();
    // A ruler click moves the mix playhead (kept when the preview stops) and leaves the preview where it is.
    frame(&mut a,&ctx,vec![],1000.0);let out=frame(&mut a,&ctx,vec![],1000.0);
    let pos=out.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape {if t.galley.text()=="0:10" && t.pos.x>200.0{Some(t.pos+t.galley.size()*0.5)}else{None}}else{None}).expect("ruler label");
    let preview=a.engine_beat();let pointer=|pressed|egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
    frame(&mut a,&ctx,vec![egui::Event::PointerMoved(pos),pointer(true)],1000.0);frame(&mut a,&ctx,vec![pointer(false)],1000.0);
    let clicked=a.beat();assert!((clicked-10.0).abs()>0.5,"the click must move the mix playhead");assert_eq!(a.engine_beat(),preview,"the click must not move the preview");
    a.stop_listen();assert_eq!(a.beat(),clicked,"stopping the preview keeps the clicked mix position");
    drop(a);std::fs::remove_dir_all(dir).ok();
}
#[test]
fn library_folder_column_compares_whole_folder_names() {
    let roots=vec!["/Music/House".to_string(),"/Music".to_string()];
    assert_eq!(super::library_ui::relative_folder("/Music/House2/x.wav",&roots),"House2");
    assert_eq!(super::library_ui::relative_folder("/Music/House/Deep/x.wav",&roots),"Deep");
    assert_eq!(super::library_ui::relative_folder("/Music/House/x.wav",&roots),"House");
    assert_eq!(super::library_ui::relative_folder("/Other/x.wav",&roots),"/Other");
}
