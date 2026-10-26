//! Library pre-listening: a temporary one-track snapshot at the file's own tempo. Never project data;
//! the mix playhead, loop and timeline view are restored when it stops.
use super::*;

pub(super) struct Listen {
    pub path:String, pub title:String, pub ready:Option<(Arc<Audio>,Arc<Analysis>)>, pub error:Option<String>,
    restore_beat:f64, restore_loop:Option<(f64,f64)>, restore_loop_on:bool,
}
/// Source seconds to preview beats on a fixed grid at the file's own tempo (stretch ratio 1).
fn start_beat(an:&Analysis)->f64 { (an.offset*an.bpm/60.0/4.0).ceil()*4.0 }
fn beat_at(an:&Analysis,sec:f64)->f64 { start_beat(an)+(sec-an.offset)*an.bpm/60.0 }
fn sec_at(an:&Analysis,beat:f64)->f64 { an.offset+(beat-start_beat(an))*60.0/an.bpm }
/// Begin at the first detected drop, else a third of the way in: where a DJ judges a track.
fn cue(an:&Analysis)->f64 {
    an.sections.iter().find(|s|s.1==2 && s.0>0.0).map(|s|an.offset+s.0*60.0/an.bpm).filter(|s|*s<an.duration-8.0).unwrap_or(an.duration/3.0)
}
impl App {
    pub(super) fn listening(&self,path:&str)->bool { self.listen.as_ref().is_some_and(|l|l.path==path) }
    /// Toggle the preview of one file: a second click stops it.
    pub(super) fn toggle_listen(&mut self,path:String,title:String,ctx:&egui::Context) {
        if self.listening(&path) { self.stop_listen(); return; }
        self.stop_comparison(); self.stop_transition_audition();
        let (restore_beat,restore_loop,restore_loop_on)=match self.listen.take() { Some(l)=>(l.restore_beat,l.restore_loop,l.restore_loop_on), None=>(self.beat(),self.active_loop,self.loop_on) };
        let _=self.out.tx.send(Cmd::Play(false)); self.set_loop(None);
        let ready=self.store.get(&path).map(|(a,an)|(a.clone(),an.clone()));
        self.listen=Some(Listen{path:path.clone(),title,ready:None,error:None,restore_beat,restore_loop,restore_loop_on});
        match ready { Some((a,an))=>self.listen_ready(&path,Ok((a,an))), None=>{
            let tx=self.tx.clone(); let ctx=ctx.clone();
            std::thread::spawn(move||{ let r=analysis::load(std::path::Path::new(&path)).map(|(a,an)|(Arc::new(a),Arc::new(an))).map_err(|e|e.to_string());
                let _=tx.send(Msg::ListenReady(path,r)); ctx.request_repaint(); });
        } }
    }
    pub(super) fn listen_ready(&mut self,path:&str,result:Result<(Arc<Audio>,Arc<Analysis>),String>) {
        let Some(l)=self.listen.as_mut().filter(|l|l.path==path) else { return; };
        match result {
            Err(e)=>{ l.error=Some(e); },
            Ok((a,an))=>{ let at=beat_at(&an,cue(&an)); l.ready=Some((a,an)); let _=self.out.snapshot(Arc::new(self.build_playback_snap())); self.seek(at); let _=self.out.tx.send(Cmd::Play(true)); },
        }
    }
    pub(super) fn stop_listen(&mut self) {
        let Some(l)=self.listen.take() else { return; };
        let _=self.out.tx.send(Cmd::Play(false)); let _=self.out.snapshot(Arc::new(self.build_playback_snap()));
        self.seek(l.restore_beat); self.loop_on=l.restore_loop_on; self.set_loop(l.restore_loop);
    }
    /// Stop when leaving the Library or at the end of the file.
    pub(super) fn validate_listen(&mut self) {
        let Some(l)=&self.listen else { return; };
        let ended=l.ready.as_ref().is_some_and(|(_,an)|self.playing() && sec_at(an,self.beat())>=an.duration-0.05);
        if self.tab!=Tab::Library || ended { self.stop_listen(); }
    }
    /// The playhead the timeline should show: the mix position stays put while a preview plays.
    pub(super) fn timeline_beat(&self)->f64 { self.listen.as_ref().map(|l|l.restore_beat).unwrap_or_else(||self.beat()) }
    pub(super) fn listen_snap(&self)->Option<Snap> {
        let (audio,an)=self.listen.as_ref()?.ready.clone()?;
        let mut mix=crate::model::Mix{output:self.mix.output,master:self.mix.master,bpm:an.bpm,..Default::default()};
        let mut t=base_track(self.listen.as_ref()?.path.clone(),&an); t.map=None; t.grid_mode=0; t.start_beat=start_beat(&an);
        mix.tracks.push(t);
        Some(Snap::build(&mix,&[Some(audio)]))
    }
    /// Preview strip: stop, title, clickable waveform with position, time.
    pub(super) fn listen_ui(&mut self,ui:&mut egui::Ui) {
        let Some(l)=&self.listen else { return; };
        let (title,error,ready)=(l.title.clone(),l.error.clone(),l.ready.clone());
        let mut stop=false; let mut seek=None;
        egui::Frame::none().fill(C::from_rgb(40,40,24)).rounding(Rounding::same(6.0)).stroke(Stroke::new(1.0_f32,C::from_rgb(110,106,30))).inner_margin(egui::Margin::symmetric(8.0,4.0)).show(ui,|ui|{
            ui.horizontal(|ui|{
                if ui.add(egui::Button::new(egui::RichText::new("⏹").color(BG)).fill(ACCENT).min_size(vec2(28.0,24.0))).on_hover_text("Stop preview (Space)").clicked() { stop=true; }
                ui.vertical(|ui|{ ui.spacing_mut().item_spacing.y=0.0;
                    ui.add(egui::Label::new(egui::RichText::new(&title).strong().size(12.5)).truncate());
                    ui.label(egui::RichText::new(match (&error,&ready) { (Some(e),_)=>format!("Cannot play: {e}"), (None,None)=>"Loading…".into(), (None,Some((_,an)))=>format!("Preview · original tempo {:.1} BPM · {}",an.bpm,an.key) }).size(10.5).color(if error.is_some() {C::LIGHT_RED} else {DIM}));
                });
                let Some((_,an))=&ready else { if error.is_none() { ui.spinner(); } return; };
                let pos=sec_at(an,self.beat()).clamp(0.0,an.duration);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    ui.label(egui::RichText::new(format!("{} / {}",fmt_t(pos),fmt_t(an.duration))).size(11.5).color(DIM).monospace());
                    let (rect,response)=ui.allocate_exact_size(vec2(ui.available_width().max(80.0),30.0),Sense::click_and_drag());
                    let painter=ui.painter_at(rect); painter.rect_filled(rect,Rounding::same(4.0),BG);
                    let n=an.peaks.len().max(1); let columns=(rect.width()/2.0) as usize;
                    for c in 0..columns { let lo=c*n/columns; let hi=((c+1)*n/columns).max(lo+1).min(n);
                        let v=an.peaks.get(lo..hi).map(|p|p.iter().map(|x|x[0].max(x[1]).max(x[2])).max().unwrap_or(0)).unwrap_or(0) as f32/255.0;
                        let x=rect.left()+c as f32*2.0+1.0; let h=(v*rect.height()*0.9).max(1.0);
                        let played=(c as f32)/(columns as f32)<=(pos/an.duration) as f32;
                        painter.line_segment([pos2(x,rect.center().y-h*0.5),pos2(x,rect.center().y+h*0.5)],Stroke::new(1.5_f32,if played {ACCENT} else {C::from_rgb(110,112,120)}));
                    }
                    let x=rect.left()+rect.width()*(pos/an.duration) as f32; painter.line_segment([pos2(x,rect.top()),pos2(x,rect.bottom())],Stroke::new(1.5_f32,TXT));
                    if let Some(p)=response.interact_pointer_pos() { if response.clicked() || response.dragged() { seek=Some(beat_at(an,((p.x-rect.left())/rect.width()).clamp(0.0,1.0) as f64*an.duration)); } }
                    response.on_hover_text("Click or drag to jump");
                });
            });
        });
        if stop { self.stop_listen(); } else if let Some(b)=seek { self.seek(b); if !self.playing() { let _=self.out.tx.send(Cmd::Play(true)); } }
    }
}
