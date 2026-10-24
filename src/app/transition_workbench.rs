//! Processed transition audition: temporary snapshot filtering, never project edits.
use super::*;

pub(super) struct Audition {
    incoming:usize, side:u8, from:f64, to:f64, repeat:bool, stamp:Vec<u64>,
    restore_loop:Option<(f64,f64)>, restore_loop_on:bool,
}
impl App {
    pub(super) fn arrangement_locked(&self)->bool {self.lock || self.mix.has_transition_locks()}
    pub(super) fn set_transition_protection(&mut self,i:usize,locked:bool) {
        if self.mix.tracks.get(i).is_none_or(|t|t.trans.locked==locked) {return;}
        self.snapshot();self.mix.tracks[i].trans.locked=locked;self.mix.sync_transition(i);
        self.drag=None;self.tl_drag=None;self.grid_drag=None;self.bm_drag=None;self.tc_drag=None;
        self.status=if locked {"Transition protected. Unlock protected transitions to move, cut or reorder the arrangement."}else{"Transition unlocked. Undo: Ctrl+Z."}.into();
    }
    pub(super) fn transition_source_ranges(&self,incoming:usize,side:usize)->Vec<(f64,f64,f64,f64)> {
        let Some((_,_,s,e))=self.mix.logical_transitions().into_iter().find(|x|x.1==incoming) else {return vec![];};
        let Some(members)=self.transition_members(incoming) else {return vec![];};
        let mut ranges=vec![];
        for &i in &members[side.min(1)] {
            let t=&self.mix.tracks[i];let g=t.grid();let lo=s.max(t.vis_start(0.0));let hi=e.min(t.end(0.0));
            if hi<=lo {continue;}
            let mut points=vec![lo];points.extend(g.cuts().filter(|b|*b>lo && *b<hi));points.push(hi);
            for w in points.windows(2) {let at=(w[0]+w[1])*0.5;ranges.push((w[0],w[1],g.region_sec(w[0],at),g.region_sec(w[1],at)));}
        }
        ranges.sort_by(|a,b|a.0.total_cmp(&b.0));ranges
    }
    pub(super) fn start_transition_audition(&mut self,i:usize,side:u8,repeat:bool) {
        let Some((_,_,s,e))=self.mix.logical_transitions().into_iter().find(|x|x.1==i) else {return;};
        let Some(stamp)=self.alignment_stamp(i) else {self.status="Load both source tracks before listening.".into();return;};
        if self.transition_members(i).is_none_or(|g|g.iter().flatten().any(|k|!self.store.contains_key(&self.mix.tracks[*k].path))) {self.status="Load every source fragment before listening.".into();return;}
        self.stop_comparison();self.stop_transition_audition();
        let from=(s-self.transition_pre as f64*4.0).max(0.0);let to=(e+self.transition_post as f64*4.0).min(self.mix.length_beats());
        if to<=from {return;}
        self.transition_audition=Some(Audition{incoming:i,side:side.clamp(1,3),from,to,repeat,stamp,restore_loop:self.active_loop,restore_loop_on:self.loop_on});
        self.loop_on=repeat;self.set_loop(if repeat {Some((from,to))}else{None});
        let _=self.out.snapshot(Arc::new(self.build_playback_snap()));self.seek(from);let _=self.out.tx.send(Cmd::Play(true));
    }
    pub(super) fn switch_transition_side(&mut self,side:u8) {
        if let Some(a)=&mut self.transition_audition {a.side=side.clamp(1,3);let _=self.out.snapshot(Arc::new(self.build_playback_snap()));}
    }
    pub(super) fn stop_transition_audition(&mut self) {
        if let Some(a)=self.transition_audition.take() {
            let _=self.out.tx.send(Cmd::Play(false));self.loop_on=a.restore_loop_on;self.set_loop(a.restore_loop);
            let _=self.out.snapshot(Arc::new(self.build_snap()));
        }
    }
    pub(super) fn validate_transition_audition(&mut self) {
        if let Some(a)=&self.transition_audition {
            if self.tab!=Tab::Transition || self.sel_in!=Some(a.incoming) || self.alignment_stamp(a.incoming).as_ref()!=Some(&a.stamp) || (!a.repeat && self.beat()>=a.to) || self.beat()<a.from-0.001 {self.stop_transition_audition();}
        }
    }
    pub(super) fn filter_transition_audition(&self,snap:&mut Snap) {
        let Some(a)=&self.transition_audition else {return;};
        if self.tab!=Tab::Transition || self.alignment_stamp(a.incoming).as_ref()!=Some(&a.stamp) {return;}
        let Some(members)=self.transition_members(a.incoming) else {return;};
        let mut loaded=0;
        for (i,t) in self.mix.tracks.iter().enumerate() {
            if !self.store.contains_key(&t.path) {continue;}
            if !members.iter().flatten().any(|k|*k==i) {snap.tracks[loaded].vis=snap.tracks[loaded].end;}
            loaded+=1;
        }
        // Keep both sides and all transition/stem/FX routing. Deck mute isolates
        // one side after its transition processing without renumbering voices.
        for d in 0..4 {
            let allowed=members.iter().enumerate().any(|(side,group)| (a.side==3 || a.side==side as u8+1) && group.iter().any(|i|self.mix.tracks[*i].lane==d));
            snap.decks[d].mute = !allowed;
            snap.decks[d].solo=false;
        }
    }
    pub(super) fn transition_tab(&mut self,ui:&mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("transition_tools").show(ui,|ui| {
            let transitions=self.mix.logical_transitions();
            if transitions.is_empty() {
                ui.label("No transition yet. Add two overlapping tracks.");
                if self.mix.has_transition_locks() && ui.button("Unlock all transitions").clicked() {self.snapshot();for t in &mut self.mix.tracks {t.trans.locked=false;}}
                return;
            }
            let mut choice=self.sel_in.and_then(|i|transitions.iter().position(|x|x.1==i)).unwrap_or(0);
            let old=choice;
            ui.horizontal_wrapped(|ui| {
                ui.strong("Transition");
                egui::ComboBox::from_id_salt("transition_choice").selected_text(format!("{} of {}",choice+1,transitions.len())).show_ui(ui,|ui| {for (k,(o,i,_,_)) in transitions.iter().enumerate() {ui.selectable_value(&mut choice,k,format!("{}: {} to {}",k+1,self.mix.tracks[*o].title,self.mix.tracks[*i].title));}});
                if ui.add_enabled(choice>0,egui::Button::new("Previous")).clicked() {choice-=1;}
                if ui.add_enabled(choice+1<transitions.len(),egui::Button::new("Next")).clicked() {choice+=1;}
            });
            if old!=choice {self.stop_transition_audition();}
            let (o,i,s,e)=transitions[choice];
            if self.transition_audition.as_ref().is_some_and(|a|a.incoming!=i) {self.stop_transition_audition();}
            self.sel_in=Some(i);
            ui.label(egui::RichText::new(format!("A: {}  |  B: {}",self.mix.tracks[o].title,self.mix.tracks[i].title)).strong());
            ui.label(format!("Overlap: mix bars {:.2}–{:.2} · {:.2} bars · {:.1} seconds",s/4.0+1.0,e/4.0+1.0,(e-s)/4.0,self.mix.secs(s,e)));
            section(ui,"Beatmatch");
            self.beatmatch_ui(ui,false);
            section(ui,"Details");
            for (side,label) in [(0,"A outgoing"),(1,"B incoming")] {
                let ranges=self.transition_source_ranges(i,side);
                let line=ranges.iter().map(|(_,_,a,b)|format!("{} – {}",source_time(*a),source_time(*b))).collect::<Vec<_>>().join("; ");
                ui.label(format!("{label} · source {line}"));
            }
            ui.horizontal_wrapped(|ui| {
                let mut protected=self.mix.tracks[i].trans.locked;
                if ui.checkbox(&mut protected,"Protect this transition").on_hover_text("Save the protection with the project. Freeze arrangement placement/ratios and this transition’s settings until unlocked.").changed() {self.set_transition_protection(i,protected);}
                if self.mix.has_transition_locks() && ui.button("Unlock all transitions").clicked() {self.snapshot();for t in &mut self.mix.tracks {t.trans.locked=false;}}
            });
            if self.mix.has_transition_locks() {ui.colored_label(C::LIGHT_YELLOW,"Protected transitions freeze arrangement moves, cuts, reorder and automatic beat ratios. Unlock to change placement.");}
            self.harmony_ui(ui,o,i);
            ui.horizontal_wrapped(|ui| {self.song_gain_control(ui,o,"A gain");self.song_gain_control(ui,i,"B gain");});
            self.transition_listen_ui(ui,i);
            section(ui,"Shape");
            ui.horizontal_wrapped(|ui| {
                ui.label("Overlap length:");
                for bars in [4,8,16,32] {
                    if ui.add_enabled(!self.arrangement_locked(),egui::Button::new(format!("{bars} bars"))).clicked() {self.snapshot();self.mix.shift_logical_transition(i,(e-s)-bars as f64*4.0,true);}
                }
                let mut ratio=self.mix.tracks[i].trans.ratio;
                ui.label("Incoming beat ratio:").on_hover_text(format!("Current mapping: one B source beat spans {:.3} mix beats. Auto selects a ratio when the arrangement is unlocked.",self.mix.tracks[i].beat_mult));
                ui.add_enabled_ui(!self.arrangement_locked(),|ui| {egui::ComboBox::from_id_salt("beat_ratio").selected_text(crate::model::RATIOS[ratio as usize].0).show_ui(ui,|ui| {for (k,(name,_)) in crate::model::RATIOS.iter().enumerate() {ui.selectable_value(&mut ratio,k as u8,*name);}});});
                if ratio!=self.mix.tracks[i].trans.ratio {self.snapshot();self.mix.tracks[i].trans.ratio=ratio;self.mix.sync_transition(i);self.mix.resolve_ratios();}
            });
            let editable=!self.lock && !self.mix.tracks[i].trans.locked;
            ui.add_enabled_ui(editable,|ui| {
                let mut preset=self.mix.tracks[i].trans.preset;
                ui.horizontal_wrapped(|ui| {ui.label("Transition preset:");egui::ComboBox::from_id_salt("transition_preset").selected_text(preset.name()).show_ui(ui,|ui| {for p in Preset::ALL {ui.selectable_value(&mut preset,p,p.name());}});});
                if preset!=self.mix.tracks[i].trans.preset {self.snapshot();let t=&mut self.mix.tracks[i].trans;t.preset=preset;t.stems=match preset {Preset::BassSwap=>[StemMode::Crossfade,StemMode::Swap,StemMode::Crossfade,StemMode::Crossfade],Preset::EchoOut=>[StemMode::Swap;4],_=>[StemMode::Crossfade;4]};}
                let mut stems=self.mix.tracks[i].trans.stems;
                ui.horizontal_wrapped(|ui| {for k in 0..4 {ui.label(STEM_N[k]);egui::ComboBox::from_id_salt(("transition_stem",k)).width(95.0).selected_text(stems[k].name()).show_ui(ui,|ui| {for mode in StemMode::ALL {ui.selectable_value(&mut stems[k],mode,mode.name());}});}});
                if stems!=self.mix.tracks[i].trans.stems {self.snapshot();self.mix.tracks[i].trans.stems=stems;}
                self.trans_curves_ui(ui,i);
            });
            self.mix.sync_transition(i);
            let stems_ready=[o,i].iter().all(|i|self.stems.contains_key(&self.mix.tracks[*i].path));
            ui.label(if stems_ready {"Stems ready: stem choices and preset processing are active."}else{"Stems unavailable: the preset volume curve is used. Separate both tracks to hear individual stem choices."});
            if !stems_ready && ui.button("Separate stems for both").clicked() {self.request_stems(o);self.request_stems(i);}
            if ui.button("Open Beatgrid ➡ source grid tools").clicked() {self.stop_transition_audition();self.sel=Some(i);self.tab=Tab::Grid;}
        });
    }
    fn transition_listen_ui(&mut self,ui:&mut egui::Ui,incoming:usize) {
        section(ui,"Listen");
        ui.horizontal_wrapped(|ui| {
            ui.label("Before:");ui.add(egui::DragValue::new(&mut self.transition_pre).range(0..=8).suffix(" bars"));
            ui.label("After:");ui.add(egui::DragValue::new(&mut self.transition_post).range(0..=8).suffix(" bars"));
            ui.checkbox(&mut self.transition_repeat,"Repeat");
            if ui.button("Play transition").clicked() {self.start_transition_audition(incoming,3,self.transition_repeat);}
            if ui.add_enabled(self.transition_audition.is_some(),egui::Button::new("Stop audition")).clicked() {self.stop_transition_audition();}
        });
        ui.horizontal_wrapped(|ui| {
            for (side,label) in [(1,"Monitor A"),(3,"Both"),(2,"Monitor B")] {
                let on=self.transition_audition.as_ref().is_some_and(|a|a.incoming==incoming && a.side==side);
                if ui.selectable_label(on,label).on_hover_text("Temporarily override deck mute/solo for audition. Keep volume, EQ, transitions, stems and effects; shared effect tails carry over when switching.").clicked() {if self.transition_audition.is_some() {self.switch_transition_side(side);}else{self.start_transition_audition(incoming,side,self.transition_repeat);}}
            }
            ui.label("Processed audition · shared FX tails continue across A/B changes");
        });
        if let Some(a)=&self.transition_audition {ui.label(format!("{} mix bars {:.2}–{:.2}. Before/after and Repeat apply when starting the next audition.",if a.repeat {"Looping"}else{"Playing once"},a.from/4.0+1.0,a.to/4.0+1.0));}
    }
}
fn source_time(s:f64)->String {format!("{}:{:06.3}",(s.max(0.0)/60.0).floor() as u64,s.max(0.0)%60.0)}
