//! Recovery workflow; autosave I/O is serialized off the UI/audio threads.
use super::*;
use crate::recovery::{self,Job,Event,Record,Entry,Worker};
use std::time::{Duration,Instant};

pub(super) struct Target {pub mix:Mix,pub file:Option<PathBuf>,pub clean:bool,pub restored:Option<PathBuf>,pub original:Option<PathBuf>}
struct Closing {body:String,token:String,ready:bool}
struct Pending {target:Target,body:String,token:String}
pub(super) struct Candidate {pub old:String,pub path:PathBuf,pub audio:Arc<Audio>,pub analysis:Arc<Analysis>,pub confirmed:bool}
pub(super) struct Persistence {
    pub worker:Worker, pub dir:PathBuf, pub enabled:bool, pub id:String,
    saved:String,last_sent:String,last_probe:Instant,last_auto:Instant,
    pub dirty:bool,pub pending_write:bool,pub last_good:Option<u64>,pub error:Option<String>,
    pub entries:Vec<Entry>,pub unreadable:usize,pub window:bool,pub show_scan:bool,pub discard:Option<PathBuf>,
    closing:Option<Closing>,pub allow_close:bool,
    pending:Option<Pending>,pub restored:Option<PathBuf>,original:Option<PathBuf>,
    pub relink_window:bool,pub relink_busy:bool,pub candidate:Option<Candidate>,pub load_errors:HashMap<String,String>,
}
impl Persistence {
    pub fn new(dir:PathBuf,mix:&Mix,enabled:bool)->Self {
        let worker=Worker::new(dir.clone()); if enabled {let _=worker.tx.send(Job::Scan);}
        let body=serde_json::to_string(mix).unwrap(); let now=Instant::now();
        Self{worker,dir,enabled,id:recovery::unique_id(),saved:body.clone(),last_sent:body,last_probe:now,last_auto:now-Duration::from_secs(15),dirty:false,pending_write:false,last_good:None,error:None,entries:vec![],unreadable:0,window:false,show_scan:enabled,discard:None,closing:None,allow_close:false,pending:None,restored:None,original:None,relink_window:false,relink_busy:false,candidate:None,load_errors:HashMap::new()}
    }
    pub fn saved_body(&self)->&str {&self.saved}
    fn path(&self)->PathBuf {self.dir.join(format!("{}.recovery.json",self.id))}
    fn record(&self,mix:&Mix,file:Option<PathBuf>)->Record {let mut r=Record::new(file.or_else(||self.original.clone()),mix.clone());r.id=self.id.clone();r}
    fn reset(&mut self,mix:&Mix,clean:bool) {
        let body=serde_json::to_string(mix).unwrap();self.id=recovery::unique_id();self.saved=if clean{body.clone()}else{String::new()};self.last_sent=if clean{body}else{String::new()};
        self.dirty=!clean;self.last_auto=Instant::now()-Duration::from_secs(15);self.last_good=None;self.error=None;self.pending_write=false;
        self.restored=None;self.original=None;self.candidate=None;self.relink_busy=false;self.load_errors.clear();
    }
    pub fn probe(&mut self,mix:&Mix,file:Option<PathBuf>,force:bool) {
        if !self.enabled || (!force && self.last_probe.elapsed()<Duration::from_secs(1)) {return;}
        self.last_probe=Instant::now();let body=serde_json::to_string(mix).unwrap();self.dirty=body!=self.saved;
        if !self.dirty {if self.last_sent!=body {let _=self.worker.tx.send(Job::Remove(self.path()));self.last_sent=body;}return;}
        if (force || self.last_auto.elapsed()>=Duration::from_secs(15)) && (force || body!=self.last_sent) {
            let _=self.worker.tx.send(Job::Write(self.record(mix,file)));self.last_sent=body;self.last_auto=Instant::now();self.pending_write=true;
        }
    }
    pub fn saved(&mut self,mix:&Mix) {
        self.saved=serde_json::to_string(mix).unwrap();self.last_sent=self.saved.clone();self.dirty=false;self.error=None;self.pending_write=false;
        let _=self.worker.tx.send(Job::Remove(self.path()));
        if let Some(path)=self.restored.take(){self.entries.retain(|e|e.path!=path);let _=self.worker.tx.send(Job::Remove(path));}
    }
}
impl App {
    pub(super) fn request_project(&mut self,target:Target) {
        if self.persistence.pending.is_some(){self.status="Protecting the current project; please wait.".into();return;}
        let body=serde_json::to_string(&self.mix).unwrap();
        if self.persistence.enabled && body!=self.persistence.saved {
            let token=recovery::unique_id();let record=self.persistence.record(&self.mix,self.file.clone());
            self.persistence.pending=Some(Pending{target,body,token:token.clone()});let _=self.persistence.worker.tx.send(Job::Protect(record,token));
            self.status="Protecting unsaved changes before switching projects…".into();
        } else {self.activate_project(target);}
    }
    fn activate_project(&mut self,target:Target) {
        self.clear_section();let _=self.out.tx.send(Cmd::Play(false));self.seek(0.0);
        self.drag=None;self.grid_drag=None;self.tl_drag=None;self.scratch_win=false;self.scratch_play=None;self.tempo_drag=None;
        self.auto_track=None;self.auto_drag=None;self.auto_solo=false;self.auto_rect=Rect::NOTHING;self.tc_drag=None;self.tc_side=0;
        self.fx_sel=None;self.fx_drag=None;self.live_fx=None;self.live_last=None;self.stem_sel=None;self.stem_drag=false;
        self.dl_offer=None;self.tl_hover=None;self.bm_drag=None;self.bm_pop=None;self.section_copy=None;self.taps.clear();self.comparison=Default::default();
        self.mix=target.mix;self.file=target.file;self.sel=None;self.sel_in=None;self.undo.clear();self.redo.clear();self.beat_check=None;
        self.persistence.reset(&self.mix,target.clean);self.persistence.restored=target.restored;self.persistence.original=target.original;
        self.load_project_sources();
        self.status=if target.clean{"Project opened".into()}else{"Recovered a copy. Save as… to keep it; the original project is untouched.".into()};
    }
    pub(super) fn finish_save(&mut self,path:PathBuf,id:&str){if id==self.persistence.id {self.save(path);}}
    pub(super) fn finish_pick(&mut self,paths:Vec<PathBuf>,sample:Option<usize>,id:&str){if id==self.persistence.id {self.load_files(paths,sample);}}
    pub(super) fn protect_close(&mut self) {
        if self.persistence.closing.is_some(){return;}
        let body=serde_json::to_string(&self.mix).unwrap();let token=recovery::unique_id();let record=self.persistence.record(&self.mix,self.file.clone());
        self.persistence.closing=Some(Closing{body,token:token.clone(),ready:false});let _=self.persistence.worker.tx.send(Job::Protect(record,token));
        self.status="Writing recovery before closing…".into();
    }
    pub(super) fn close_if_ready(&mut self,ctx:&egui::Context){
        if let Some(close)=&self.persistence.closing {if close.ready {
            if close.body==serde_json::to_string(&self.mix).unwrap(){self.persistence.allow_close=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
            else {self.persistence.allow_close=false;ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);self.persistence.closing=None;self.protect_close();}
        }}
    }
    pub(super) fn new_project(&mut self){self.request_project(Target{mix:Mix::default(),file:None,clean:true,restored:None,original:None});}
    fn load_project_sources(&mut self) {
        let paths:std::collections::BTreeSet<_>=self.mix.tracks.iter().map(|t|t.path.clone()).filter(|p|!self.store.contains_key(p)).collect();
        self.loading+=paths.len();let tx=self.tx.clone();let docid=self.persistence.id.clone();std::thread::spawn(move ||for p in paths {let pb=PathBuf::from(p);let result=analysis::load(&pb).map_err(|e|e.to_string());let _=tx.send(Msg::Loaded(pb,result,Some((usize::MAX,0.0)),docid.clone()));});
    }
    pub(super) fn poll_persistence(&mut self) {
        let events:Vec<_>=self.persistence.worker.rx.try_iter().collect();
        for event in events {match event {
            Event::Protected(token,result)=>{
                if self.persistence.closing.as_ref().is_some_and(|c|c.token==token) {
                    match result {Ok(())=>self.persistence.closing.as_mut().unwrap().ready=true,Err(e)=>{self.persistence.closing=None;self.persistence.error=Some(e.clone());self.status=format!("App kept open: could not protect unsaved changes ({e})");}}
                    continue;
                }
                if self.persistence.pending.as_ref().is_none_or(|p|p.token!=token){continue;}
                let pending=self.persistence.pending.take().unwrap();
                match result {Ok(())=>{if serde_json::to_string(&self.mix).unwrap()!=pending.body {self.request_project(pending.target);} else {self.activate_project(pending.target);}},Err(e)=>{self.persistence.error=Some(e.clone());self.status=format!("Project kept open: could not protect unsaved changes ({e})");}}
            },
            Event::Listed(entries,bad)=>{self.persistence.entries=entries.into_iter().filter(|e|e.record.id!=self.persistence.id).collect();self.persistence.unreadable=bad;
                if self.persistence.show_scan && (!self.persistence.entries.is_empty()||bad>0){self.persistence.window=true;}self.persistence.show_scan=false;},
            Event::Written(id,at)=>if id==self.persistence.id {self.persistence.last_good=Some(at);self.persistence.pending_write=false;self.persistence.error=None;},
            Event::Restored(path,record)=>{self.request_project(Target{mix:record.mix,file:None,clean:false,restored:Some(path),original:record.original});self.persistence.window=false;},
            Event::Error(e)=>{self.persistence.pending_write=false;self.persistence.last_sent.clear();self.persistence.last_auto=Instant::now();self.persistence.error=Some(e.clone());self.status=format!("Recovery error: {e}");},
        }}
        self.persistence.probe(&self.mix,self.file.clone(),false);
    }
    pub(super) fn missing_sources(&self)->Vec<String> {self.mix.tracks.iter().map(|t|t.path.clone()).filter(|p|!self.store.contains_key(p)).collect::<std::collections::BTreeSet<_>>().into_iter().collect()}
    fn choose_relink(&mut self,old:String) {
        if self.persistence.relink_busy{return;}self.persistence.relink_busy=true;self.persistence.candidate=None;
        let id=self.persistence.id.clone();let tx=self.tx.clone();std::thread::spawn(move ||{
            let req=crate::dialogs::Req::Open{title:format!("Locate original recording: {old}"),filter:("Audio".into(),crate::library::EXTS.iter().map(|s|s.to_string()).collect())};
            let result=crate::dialogs::run(req).and_then(|paths|match paths.and_then(|p|p.into_iter().next()){None=>Ok(None),Some(path)=>analysis::load(&path).map(|(a,n)|Some((path,a,n))).map_err(|e|e.to_string())});
            let _=tx.send(Msg::RelinkReady(id,old,result));
        });
    }
    pub(super) fn apply_relink(&mut self) {
        let Some(candidate)=self.persistence.candidate.as_ref() else{return;};if !candidate.confirmed{return;}
        let required=self.mix.tracks.iter().filter(|t|t.path==candidate.old).map(|t|t.sections.iter().map(|p|p.1).fold(t.duration-t.trim_out,f64::max)).fold(0.0,f64::max);
        if candidate.audio.secs()+0.025<required {self.status="Replacement is shorter than source audio used by this project. Choose the original recording.".into();return;}
        let path=candidate.path.display().to_string();let mut mix=self.mix.clone();
        match recovery::relink(&mut mix,&candidate.old,&path) {
            Ok(n)=>{let c=self.persistence.candidate.take().unwrap();self.snapshot();self.clear_section();self.mix=mix;
                self.store.insert(path.clone(),(c.audio,c.analysis));if let Some(stems)=self.stems.get(&c.old).cloned(){self.stems.insert(path,stems);}
                self.persistence.load_errors.remove(&c.old);self.status=format!("Relinked {n} clip(s); grid and edits retained. Save the project to keep the new path.");},
            Err(e)=>self.status=e,
        }
    }
    pub(super) fn persistence_windows(&mut self,ctx:&egui::Context) {
        if self.persistence.closing.is_some(){egui::Window::new("Saving recovery before closing").collapsible(false).show(ctx,|ui|{ui.spinner();ui.label("Waiting for storage to confirm the recovery copy…");if ui.button("Cancel closing").clicked(){self.persistence.allow_close=false;ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);self.persistence.closing=None;}});}

        if self.persistence.window {let mut open=true;let mut restore=None;let mut discard=None;
            egui::Window::new("Recover unsaved projects").open(&mut open).default_width(650.0).show(ctx,|ui|{
                ui.label("Autosaved copies are separate from your project files. Restore opens a copy; Save as… keeps it.");
                if self.persistence.unreadable>0 {ui.colored_label(C::LIGHT_YELLOW,format!("{} recovery file(s) could not be read and were left untouched.",self.persistence.unreadable));}
                egui::ScrollArea::vertical().max_height(320.0).show(ui,|ui|{for e in &self.persistence.entries {
                    ui.separator();ui.strong(&e.record.mix.name);ui.label(format!("{} clips · saved {} seconds ago",e.record.mix.tracks.len(),recovery::now().saturating_sub(e.record.saved_at)));
                    ui.label(egui::RichText::new(e.record.original.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"Never saved to a project file".into())).color(DIM));
                    ui.horizontal(|ui|{if ui.button("Restore copy").clicked(){restore=Some(e.path.clone());}if ui.button("Discard…").clicked(){discard=Some(e.path.clone());}});
                }});
                if self.persistence.entries.is_empty(){ui.label("No recoverable copies found.");}
                if ui.button("Keep for later").clicked(){self.persistence.window=false;}
            });self.persistence.window&=open;
            if let Some(path)=restore {let _=self.persistence.worker.tx.send(Job::Read(path));}
            if let Some(path)=discard {self.persistence.discard=Some(path);}
        }
        if let Some(path)=self.persistence.discard.clone(){egui::Window::new("Delete this recovery copy?").collapsible(false).show(ctx,|ui|{
            ui.label("This deletes only the selected autosave. The original project and audio files are kept.");
            ui.label(path.display().to_string());ui.horizontal(|ui|{
                if ui.button("Delete recovery copy").clicked(){let _=self.persistence.worker.tx.send(Job::Remove(path.clone()));self.persistence.entries.retain(|e|e.path!=path);self.persistence.discard=None;}
                if ui.button("Cancel").clicked(){self.persistence.discard=None;}
            });
        });}
        if self.persistence.relink_window {let mut open=true;egui::Window::new("Locate missing audio").open(&mut open).default_width(700.0).show(ctx,|ui|{
            ui.label("Choose the same recording at its new location. Existing grid, clip positions and section edits are kept.");
            let missing=self.missing_sources();if missing.is_empty(){ui.label("All project sources are loaded.");}
            egui::ScrollArea::vertical().max_height(260.0).show(ui,|ui|{for path in missing {ui.separator();ui.label(&path);if let Some(error)=self.persistence.load_errors.get(&path){ui.colored_label(C::LIGHT_YELLOW,error);}
                if ui.add_enabled(!self.persistence.relink_busy,egui::Button::new("Locate file…")).clicked(){self.choose_relink(path);}
            }});
            if self.persistence.relink_busy {ui.spinner();ui.label("Reading replacement audio…");}
            if let Some(c)=&mut self.persistence.candidate {ui.separator();ui.label(format!("Replacement: {} · {:.2} seconds",c.path.display(),c.audio.secs()));
                ui.checkbox(&mut c.confirmed,"I confirm this is the same recording (not a different edit or remix)");
                if ui.add_enabled(c.confirmed,egui::Button::new("Use replacement · keep grid and edits")).clicked(){self.apply_relink();}
            }
        });self.persistence.relink_window=open;}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autosave_reuses_one_document_record_and_throttles_unchanged_or_recent_writes() {
        let dir=std::env::temp_dir().join(format!("mixstudio-autosave-{}",recovery::unique_id()));let mut mix=Mix::default();let mut p=Persistence::new(dir.clone(),&mix,true);
        mix.name="First edit".into();p.last_probe=Instant::now()-Duration::from_secs(2);p.probe(&mix,None,false);p.worker.flush();
        let path=p.path();assert_eq!(recovery::read_record(&path).unwrap().mix.name,"First edit");
        mix.name="Second edit".into();p.last_probe=Instant::now()-Duration::from_secs(2);p.probe(&mix,None,false);p.worker.flush();assert_eq!(recovery::read_record(&path).unwrap().mix.name,"First edit");
        p.last_auto=Instant::now()-Duration::from_secs(16);p.last_probe=Instant::now()-Duration::from_secs(2);p.probe(&mix,None,false);p.worker.flush();assert_eq!(recovery::read_record(&path).unwrap().mix.name,"Second edit");
        assert_eq!(recovery::scan(&dir).0.len(),1);p.saved(&mix);p.worker.flush();assert!(!path.exists());drop(p);std::fs::remove_dir_all(dir).unwrap();
    }
}
