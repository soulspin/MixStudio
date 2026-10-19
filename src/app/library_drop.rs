//! A library drag captures its destination before asynchronous decoding starts.
use super::*;
#[derive(Clone,Copy,Debug,PartialEq)]
pub(super) struct Target {pub lane:usize,pub beat:f64}
pub(super) struct Geometry {pub left:f32,pub right:f32,pub ppb:f32,pub scroll:f64,pub rows:Vec<(usize,Rect)>}
pub(super) type LoadedRows=Vec<(PathBuf,Result<(Audio,Analysis,bool),String>)>;
impl App {
    pub(super) fn library_drop_target(&self,pos:egui::Pos2)->Option<Target> {
        if self.arrangement_locked(){return None;}
        let g=self.library_ui.geometry.as_ref()?;
        if pos.x<g.left||pos.x>g.right{return None;}
        let lane=g.rows.iter().find(|(lane,r)|*lane<4&&r.contains(pos)).map(|(lane,_)|*lane)?;
        let beat=self.snap_beat(g.scroll+((pos.x-g.left)/g.ppb) as f64).max(0.0);Some(Target{lane,beat})
    }
    pub(super) fn library_drag_frame(&mut self,ctx:&egui::Context) {
        let Some(payload)=egui::DragAndDrop::payload::<LibDrag>(ctx)else{return;};
        if ctx.input(|i|i.key_pressed(egui::Key::Escape)){egui::DragAndDrop::clear_payload(ctx);return;}
        let target=ctx.pointer_latest_pos().and_then(|p|self.library_drop_target(p));
        if let (Some(target),Some(g))=(target,self.library_ui.geometry.as_ref()) {
            let x=g.left+((target.beat-g.scroll) as f32)*g.ppb;
            if let Some((_,row))=g.rows.iter().find(|(l,_)|*l==target.lane) {
                let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground,egui::Id::new("library_drop_preview")));
                painter.rect_stroke(*row,0.0,Stroke::new(2.0_f32,ACCENT));painter.line_segment([pos2(x,row.top()),pos2(x,row.bottom())],Stroke::new(3.0_f32,ACCENT));
            }
        }
        egui::show_tooltip_at_pointer(ctx,egui::LayerId::new(egui::Order::Tooltip,egui::Id::new("library_drag_tip")),egui::Id::new("library_drag_tip"),|ui|{
            ui.label(target.map(|t|format!("{} file(s) → {} {} · {} at bar {:.0}, beat {:.0}{}",payload.0.len(),if t.lane<2{"Deck"}else{"Sample"},if t.lane<2{t.lane+1}else{t.lane-1},if t.lane<2{"downbeat"}else{"clip start"},(t.beat/4.0).floor()+1.0,t.beat.rem_euclid(4.0).floor()+1.0,if t.lane<2{" (intro kept at timeline start)"}else{""})).unwrap_or_else(||"Drop on a deck or sample lane".into()));
        });
        if ctx.input(|i|i.pointer.primary_released()) {egui::DragAndDrop::clear_payload(ctx);if let Some(target)=target{self.request_library_batch(payload.0.clone(),Some(target));}}
    }
    pub(super) fn request_library_batch(&mut self,paths:Vec<String>,target:Option<Target>) {
        if self.arrangement_locked(){self.status="Unlock the timeline before adding library files.".into();return;}
        let mut seen=std::collections::HashSet::new();let paths:Vec<_>=paths.into_iter().filter(|p|seen.insert(p.clone())).collect();if paths.is_empty(){return;}
        if target.is_some_and(|t|t.lane>=4||!t.beat.is_finite()||t.beat<0.0){return;}
        self.loading+=paths.len();self.status=format!("Loading {} library file(s)…",paths.len());
        let tx=self.tx.clone();let id=self.persistence.id.clone();
        std::thread::spawn(move||{let rows=paths.into_iter().map(|p|{let path=PathBuf::from(p);let result=analysis::load(&path).map(|(audio,an)|{let cached=analysis::cache_key(&path).map(|k|crate::stems::cache_file(&k).exists()).unwrap_or(false);(audio,an,cached)}).map_err(|e|e.to_string());(path,result)}).collect();let _=tx.send(Msg::LibraryBatch(id,target,rows));});
    }
    pub(super) fn finish_library_batch(&mut self,id:&str,target:Option<Target>,rows:LoadedRows) {
        self.loading=self.loading.saturating_sub(rows.len());if id!=self.persistence.id{return;}
        if self.arrangement_locked(){self.status="Library import cancelled: timeline was locked while loading.".into();return;}
        let mut ready=vec![];let mut failures=vec![];
        for (path,result) in rows {match result {Ok((audio,an,cached)) if an.duration.is_finite()&&an.duration>0.0&&an.bpm.is_finite()&&an.bpm>0.0=>ready.push((path,audio,an,cached)),Ok(_)=>failures.push(format!("{}: invalid audio analysis",path.display())),Err(e)=>failures.push(format!("{}: {e}",path.display()))}}
        if ready.is_empty(){self.status=format!("No files added. {}",failures.join("; "));return;}
        self.snapshot();let count=ready.len();let mut cursor=target.map(|t|t.beat).unwrap_or(0.0);let mut selected=None;
        if target.is_some_and(|t|t.lane<2)&&self.mix.main_order().is_empty(){self.mix.bpm=ready[0].2.bpm.round();}
        for (position,(path,audio,an,cached)) in ready.into_iter().enumerate() {
            let lane=target.map(|t|if t.lane<2{(t.lane+position)%2}else{t.lane});
            self.insert_cached(path,audio,an,lane.map(|l|(l,cursor)),cached);
            if target.is_none(){self.mix.resolve_ratios();}
            let i=self.mix.tracks.len()-1;
            if let Some(lane)=lane {
                let reference=self.mix.bpm_at(cursor);let track=&mut self.mix.tracks[i];
                let multiplier=crate::model::auto_mult(track.bpm,reference);track.beat_mult=multiplier;
                track.trans.ratio=crate::model::RATIOS.iter().position(|(_,m)|m.is_some_and(|m|(m-multiplier).abs()<1e-8)).unwrap_or(1) as u8;
                // Decks align their analysed downbeat, not the file edge: leading
                // audio is rarely an exact beat/bar. Samples retain exact edge placement.
                if lane<2 {
                    track.start_beat=cursor;
                    let missing=(-track.vis_start(0.0)).max(0.0);
                    let step=match self.snapping {Snapping::Off=>0.0,Snapping::Free=>1.0,Snapping::Half=>2.0,Snapping::Full=>4.0};
                    // Keep pre-downbeat audio accessible at the timeline's left boundary.
                    if missing>0.0 {track.start_beat+=if step>0.0 {(missing/step).ceil()*step}else{missing};}
                } else {track.start_beat+=cursor-track.vis_start(0.0);}
                let length=track.end(0.0)-track.vis_start(0.0);
                cursor=track.end(0.0)-if lane<2{(self.bars as f64*4.0).min(length/2.0)}else{0.0};
                if lane<2 {cursor=self.snap_beat(cursor).max(0.0);}
            }
            selected=Some(i);
        }
        self.sel=selected;self.clear_section();self.samples=Default::default();
        self.status=if failures.is_empty(){format!("Added {count} file(s){}. Ctrl+Z undoes the import.",if target.is_some(){" at the drop position"}else{" to the end of the mix"})}else{format!("Added {count} file(s); {} failed: {}",failures.len(),failures.join("; "))};
    }
}
