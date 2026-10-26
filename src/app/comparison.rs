//! Transition workbench. Audition state is temporary and never enters project/export data.
use super::*;
use crate::gridtools::Anchor;

pub(super) struct Comparison {
    cache:std::cell::RefCell<Option<(Vec<u64>,Track)>>,
    pub incoming:Option<usize>, pub stamp:Vec<u64>, pub center:f64, pub span:f64,
    pub anchors:[Option<Anchor>;2], pub slot:usize, pub target:f64,
    pub proposed:bool, pub audition:u8, pub metro:bool, pub preview_end:Option<f64>, pub restore_loop:Option<(f64,f64)>,
}
impl Default for Comparison {
    fn default()->Self { Self{cache:Default::default(),incoming:None,stamp:vec![],center:0.0,span:8.0,anchors:[None,None],slot:0,target:0.0,proposed:true,audition:0,metro:false,preview_end:None,restore_loop:None} }
}
impl App {
    pub(super) fn transition_members(&self,incoming:usize)->Option<[Vec<usize>;2]> {
        let (o,i,s,e)=self.mix.logical_transitions().into_iter().find(|x|x.1==incoming)?;
        let same=|a:usize,b:usize| {
            let (a,b)=(&self.mix.tracks[a],&self.mix.tracks[b]);
            a.trans.fragment_group!=0 && a.trans.fragment_group==b.trans.fragment_group && a.path==b.path && a.lane==b.lane
        };
        let mut members=[vec![],vec![]];
        for (a,b,from,to) in self.mix.transitions() {
            if from==s && to==e && (a==o || same(a,o)) && (b==i || same(b,i)) {
                if !members[0].contains(&a) {members[0].push(a);}
                if !members[1].contains(&b) {members[1].push(b);}
            }
        }
        Some(members)
    }
    pub(super) fn open_comparison(&mut self,incoming:usize) {
        let Some((_,_,s,e))=self.mix.logical_transitions().into_iter().find(|x|x.1==incoming) else { return; };
        self.stop_transition_audition(); self.stop_comparison(); self.sel_in=Some(incoming); self.sel=Some(incoming); self.tab=Tab::Compare;
        self.comparison=Comparison{incoming:Some(incoming),stamp:self.alignment_stamp(incoming).unwrap_or_default(),center:(s+e)*0.5,target:((s+e)*0.5).round(),..Default::default()};
    }
    pub(super) fn stop_comparison(&mut self) {
        if self.comparison.audition!=0 {
            let _=self.out.tx.send(Cmd::Play(false));
            let range=self.comparison.restore_loop.take(); self.set_loop(range);
            self.comparison.restore_loop=None;
        }
        self.comparison.audition=0; self.comparison.preview_end=None;
    }
    pub(super) fn validate_comparison(&mut self) {
        let valid=self.comparison.incoming.and_then(|i|self.alignment_stamp(i)).as_ref()==Some(&self.comparison.stamp);
        if self.comparison.incoming.is_some() && (!valid || self.tab!=Tab::Compare) {
            self.stop_comparison();
            if !valid { self.comparison.anchors=[None,None]; self.comparison.stamp.clear(); }
        }
        if self.comparison.preview_end.is_some_and(|end|self.beat()>=end) { self.stop_comparison(); }
    }
    pub(super) fn build_playback_snap(&self)->Snap {
        if let Some(snap)=self.listen_snap() { return snap; }
        let mut normal=self.build_snap();
        self.filter_transition_audition(&mut normal);
        if self.tab!=Tab::Compare { return normal; }
        normal.metro=if self.comparison.metro {Some(self.mix.metro_vol)} else {None};
        if self.comparison.audition==0 { return normal; }
        let Some(i)=self.comparison.incoming else { return normal; };
        if self.alignment_stamp(i).as_ref()!=Some(&self.comparison.stamp) { return normal; }
        let Some(members)=self.transition_members(i) else {return normal;};
        let indices=match self.comparison.audition {1=>members[0].clone(),2=>members[1].clone(),_=>members.into_iter().flatten().collect()};
        let proposed=self.proposed_track();
        let mut mix=self.mix.clone(); mix.tracks=indices.iter().map(|&k|if k==i {proposed.clone().unwrap_or_else(||self.mix.tracks[k].clone())} else {self.mix.tracks[k].clone()}).collect();
        // Hear source attacks before transition curves, EQ, FX and stem automation.
        mix.decks=[crate::model::Deck::default();4]; mix.fx.clear();
        for t in &mut mix.tracks { t.auto.clear(); t.stem_mutes.clear(); }
        let audio:Vec<_>=mix.tracks.iter().map(|t|self.store.get(&t.path).map(|x|x.0.clone())).collect();
        let mut dry=Snap::build(&mix,&audio); dry.trans.clear(); dry.tcurves.clear(); dry.tempo=normal.tempo; dry.metro=normal.metro; dry
    }
    pub(super) fn start_comparison_preview(&mut self,side:u8,from:f64,to:f64) {
        self.stop_transition_audition();
        if from>=to { return; }
        if self.comparison.audition==0 {
            self.comparison.restore_loop=self.active_loop;
            self.set_loop(None);
        }
        self.comparison.audition=side; self.comparison.preview_end=Some(to);
        // Queue the audition snapshot before playback, avoiding a frame of the full mix.
        let _=self.out.snapshot(Arc::new(self.build_playback_snap()));
        self.seek(from.max(0.0)); let _=self.out.tx.send(Cmd::Play(true));
    }
    fn proposed_track(&self)->Option<Track> {
        if !self.comparison.proposed { return None; }
        let i=self.comparison.incoming?;
        if self.transition_members(i)?.iter().any(|group|group.len()>1) {return None;}
        if self.alignment_stamp(i).as_ref()!=Some(&self.comparison.stamp) { return None; }
        let [Some(a),Some(b)]=self.comparison.anchors else { return None; };
        let mut key=self.comparison.stamp.clone(); key.extend([a.beat.to_bits(),a.source.to_bits(),b.beat.to_bits(),b.source.to_bits()]);
        if let Some((saved,track))=self.comparison.cache.borrow().as_ref() { if *saved==key {
            // Cache timing only: current stretch, fades and metadata remain authoritative.
            let mut current=self.mix.tracks.get(i)?.clone(); current.bpm=track.bpm; current.offset=track.offset;
            current.start_beat=track.start_beat; current.map=track.map.clone(); current.grid_mode=track.grid_mode;
            return Some(current);
        } }
        let track=crate::gridtools::anchored(self.mix.tracks.get(i)?,a,b).ok()?.0;
        *self.comparison.cache.borrow_mut()=Some((key,track.clone())); Some(track)
    }
    pub(super) fn apply_comparison_anchors(&mut self) {
        if self.arrangement_locked() {return;}
        let Some(i)=self.comparison.incoming else { return; };
        if self.transition_members(i).is_some_and(|groups|groups.iter().any(|g|g.len()>1)) {self.status="Correct source grids before splitting clips.".into();return;}
        if self.alignment_stamp(i).as_ref()!=Some(&self.comparison.stamp) { self.validate_comparison(); self.status="Timing changed. Set the anchors again.".into(); return; }
        let [Some(a),Some(b)]=self.comparison.anchors else { return; };
        match crate::gridtools::anchored(&self.mix.tracks[i],a,b) {
            Ok((track,p))=>{ self.stop_comparison(); self.snapshot(); self.mix.tracks[i]=track; self.beat_check=None;
                self.comparison.anchors=[None,None]; self.comparison.stamp=self.alignment_stamp(i).unwrap_or_default();
                self.status=format!("Two-anchor correction: {:.3} BPM ({:+.3}%). Undo: Ctrl+Z.",p.bpm,p.percent);
            }, Err(e)=>self.status=e,
        }
    }
    pub(super) fn comparison_tab(&mut self,ui:&mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("comparison_workbench").show(ui,|ui| {
            self.beatmatch_ui(ui,true);
            let Some(i)=self.sel_in.filter(|i|self.mix.logical_transitions().iter().any(|x|x.1==*i)) else { return; };
            if self.comparison.incoming!=Some(i) { self.open_comparison(i); }
            if self.alignment_stamp(i).as_ref()!=Some(&self.comparison.stamp) {
                self.stop_comparison(); self.comparison.anchors=[None,None]; self.comparison.stamp=self.alignment_stamp(i).unwrap_or_default();
            }
            let Some((o,i,s,e))=self.mix.logical_transitions().into_iter().find(|x|x.1==i) else { return; };
            ui.horizontal_wrapped(|ui| {
                ui.strong("Compare attacks");
                if ui.button("⬅ 4 beats").clicked(){self.comparison.center-=4.0;}
                if ui.button("4 beats ➡").clicked(){self.comparison.center+=4.0;}
                if ui.button("At playhead").clicked(){self.comparison.center=self.beat();}
                ui.label("View:"); for span in [2.0,4.0,8.0,16.0,32.0] { ui.selectable_value(&mut self.comparison.span,span,format!("{span:.0} beats")); }
                ui.checkbox(&mut self.comparison.metro,"Preview metronome");
                if self.comparison.anchors.iter().all(Option::is_some) {ui.checkbox(&mut self.comparison.proposed,"Preview proposed grid");}
            });
            self.comparison.center=self.comparison.center.clamp(s,e);
            let half=self.comparison.span*0.5; let from=(self.comparison.center-half).max(s); let to=(self.comparison.center+half).min(e);
            if to-from<0.01 { ui.label("No visible overlap."); return; }
            let members=self.transition_members(i).unwrap_or([vec![o],vec![i]]);
            let editable=!self.arrangement_locked() && members.iter().all(|g|g.len()==1) && self.mix.tracks[i].sections.is_empty() && (self.mix.tracks[i].beat_mult-1.0).abs()<1e-9;
            ui.horizontal_wrapped(|ui| {
                for (side,label) in [(1,"Listen A"),(2,"Listen B"),(3,"Listen both")] {
                    if ui.selectable_label(self.comparison.audition==side,label).clicked(){self.start_comparison_preview(side,from,to);}
                }
                if ui.button("Stop preview").clicked(){self.stop_comparison();}
                ui.label(egui::RichText::new("Dry preview before transitions / FX · stops at view end").color(DIM).size(11.0));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Incoming anchor:"); ui.selectable_value(&mut self.comparison.slot,0,"Set A"); ui.selectable_value(&mut self.comparison.slot,1,"Set B");
                ui.label("Target mix beat:"); let mut number=self.comparison.target+1.0;
                if ui.add(egui::DragValue::new(&mut number).range(s.ceil()+1.0..=e.floor()+1.0).speed(1.0).fixed_decimals(0)).changed(){self.comparison.target=number.round()-1.0;}
                if ui.button("Target at playhead").clicked(){self.comparison.target=self.beat().round().clamp(s.ceil(),e.floor());}
                if ui.button("Clear anchors").clicked(){self.comparison.anchors=[None,None];}
            });
            ui.label(egui::RichText::new("Choose a target beat, then click its incoming attack below. Set A near the start and B later (at least 4 beats apart). Yellow areas need manual checking.").color(DIM).size(11.0));
            let evidence=self.beat_check.as_ref().filter(|c|c.incoming==i && self.alignment_stamp(i).as_ref()==Some(&c.stamp)).map(|c|c.evidence.clone()).unwrap_or_default();
            let (rect,_)=ui.allocate_exact_size(vec2(ui.available_width().max(100.0),152.0),Sense::hover());
            let label_width=120.0; let wave=Rect::from_min_max(pos2(rect.left()+label_width,rect.top()),rect.max);
            let x=|beat:f64|wave.left()+((beat-from)/(to-from)) as f32*wave.width();
            let painter=ui.painter().with_clip_rect(rect);
            let proposed=self.proposed_track();
            for (row,ti) in [o,i].into_iter().enumerate() {
                let r=Rect::from_min_max(pos2(wave.left(),rect.top()+row as f32*76.0),pos2(wave.right(),rect.top()+(row+1) as f32*76.0-4.0));
                painter.rect_filled(r,3.0,BG);
                painter.text(pos2(rect.left()+4.0,r.center().y),Align2::LEFT_CENTER,format!("{} · {}",if row==0{"A outgoing"}else{"B incoming"},self.mix.tracks[ti].lane+1),FontId::proportional(12.0),if row==0{BLUE}else{ACCENT});
                for ev in &evidence { if !ev.reliable && ev.beat>=from && ev.beat<=to {
                    painter.rect_filled(Rect::from_min_max(pos2(x((ev.beat-0.45).max(from)),r.top()),pos2(x((ev.beat+0.45).min(to)),r.bottom())),0.0,C::from_rgba_unmultiplied(230,170,30,32));
                } }
                let t=if row==1 {proposed.as_ref().unwrap_or(&self.mix.tracks[ti])}else{&self.mix.tracks[ti]}; let grid=t.grid();
                let grids:Vec<_>=if row==1 && proposed.is_some() {vec![(t.vis_start(0.0),t.end(0.0),grid.clone())]} else {
                    members[row].iter().map(|&k|{let t=&self.mix.tracks[k];(t.vis_start(0.0),t.end(0.0),t.grid())}).collect()
                };
                if let Some((audio,_))=self.store.get(&t.path) {
                    let n=r.width().max(1.0) as usize;
                    for px in 0..n { let b0=from+(to-from)*px as f64/n as f64; let b1=from+(to-from)*(px+1) as f64/n as f64;
                        let Some((_,end,grid))=grids.iter().find(|(a,b,_)|b0>=*a && b0<*b) else {continue;};
                        let b1=b1.min(*end);
                        let a=(grid.sec(b0)*audio.sr as f64).max(0.0) as usize; let b=(grid.region_sec(b1,b0)*audio.sr as f64).max(0.0) as usize;
                        let end=b.max(a+1).min(audio.l.len().min(audio.r.len())); let step=((end.saturating_sub(a))/64).max(1);
                        let peak=(a.min(end)..end).step_by(step).map(|k|audio.l[k].abs().max(audio.r[k].abs())).fold(0.0_f32,f32::max).min(1.0);
                        let height=peak*r.height()*0.45; let xx=r.left()+px as f32;
                        painter.line_segment([pos2(xx,r.center().y-height),pos2(xx,r.center().y+height)],Stroke::new(1.0_f32,if row==0{BLUE}else{ACCENT}));
                    }
                }
                if row==1 {
                    let response=ui.interact(r,ui.id().with("incoming_anchor_wave"),Sense::click());
                    if editable && response.clicked() { if let Some(p)=response.interact_pointer_pos() {
                        let beat=from+(p.x-r.left()) as f64/r.width() as f64*(to-from);
                        self.comparison.anchors[self.comparison.slot]=Some(Anchor{beat:self.comparison.target.round(),source:grid.sec(beat)});
                        if self.comparison.slot==0 { self.comparison.slot=1; self.comparison.target=(self.comparison.target+16.0).min(e.floor()); }
                    } }
                }
            }
            for b in from.ceil() as i64..=to.floor() as i64 { let xx=x(b as f64);
                painter.line_segment([pos2(xx,rect.top()),pos2(xx,rect.bottom())],Stroke::new(1.0_f32,if b%4==0{C::from_rgb(230,90,90)}else{LINE}));
                painter.text(pos2(xx+3.0,rect.top()+2.0),Align2::LEFT_TOP,format!("{}",b+1),FontId::proportional(10.0),TXT);
            }
            for ev in &evidence { if ev.reliable && ev.beat>=from && ev.beat<=to { painter.circle_filled(pos2(x(ev.beat),rect.bottom()-5.0),3.0,if ev.percussion{C::from_rgb(100,210,220)}else{C::LIGHT_GREEN}); } }
            for (slot,a) in self.comparison.anchors.iter().enumerate() { if let Some(a)=a {
                let b=proposed.as_ref().unwrap_or(&self.mix.tracks[i]).grid().beat(a.source); if b>=from && b<=to {
                    painter.line_segment([pos2(x(b),rect.center().y),pos2(x(b),rect.bottom())],Stroke::new(2.0_f32,C::WHITE));
                    painter.text(pos2(x(b)+3.0,rect.center().y+12.0),Align2::LEFT_TOP,if slot==0{"A"}else{"B"},FontId::proportional(12.0),C::WHITE);
                }
            } }
            if self.beat()>=from && self.beat()<=to { let xx=x(self.beat()); painter.line_segment([pos2(xx,rect.top()),pos2(xx,rect.bottom())],Stroke::new(1.5_f32,C::WHITE)); }
            ui.horizontal_wrapped(|ui| {
                for (slot,a) in self.comparison.anchors.iter_mut().enumerate() { if let Some(a)=a {
                    ui.label(format!("{}: beat {:.0} ⬅ source",if slot==0{"A"}else{"B"},a.beat+1.0));
                    ui.add_enabled(editable,egui::DragValue::new(&mut a.source).speed(0.001).fixed_decimals(3).suffix(" s"));
                } }
                if let [Some(a),Some(b)]=self.comparison.anchors { match crate::gridtools::anchored(&self.mix.tracks[i],a,b) {
                    Ok((_,p))=>{ui.label(format!("Preview: {:.3} ➡ {:.3} BPM ({:+.3}%)",self.mix.tracks[i].bpm,p.bpm,p.percent)); if ui.add_enabled(editable,egui::Button::new("Apply both anchors")).clicked(){self.apply_comparison_anchors();}},
                    Err(e)=>{ui.colored_label(C::LIGHT_YELLOW,e);},
                } }
            });
            if !editable { ui.colored_label(C::LIGHT_YELLOW,"Anchor correction needs unlocked editing and an uncut incoming clip at 1:1 ratio."); }
            ui.label(egui::RichText::new("Green: paired bass attacks · cyan: percussion evidence · yellow: weak / conflicting evidence · unmarked: not measured. Anchors preserve the grid's local shape; Undo restores the previous grid.").color(DIM).size(11.0));
        });
    }
}
