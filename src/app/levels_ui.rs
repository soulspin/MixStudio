//! Saved song/master controls; source measurements run outside the UI thread.
use super::*;
impl App {
    pub(super) fn set_song_gain(&mut self,i:usize,db:f32) {
        if !db.is_finite() || !(-24.0..=12.0).contains(&db) {return;}
        let Some(t)=self.mix.tracks.get(i) else {return;};if t.gain_db==db {return;}
        let group=t.trans.fragment_group;let path=t.path.clone();self.snapshot();
        for (k,t) in self.mix.tracks.iter_mut().enumerate() {if k==i || (group!=0 && t.trans.fragment_group==group && t.path==path) {t.gain_db=db;}}
    }
    pub(super) fn song_gain_control(&mut self,ui:&mut egui::Ui,i:usize,label:&str) {
        let Some(t)=self.mix.tracks.get(i) else {return;};let mut db=t.gain_db;
        ui.label(label);let response=ui.add(egui::DragValue::new(&mut db).range(-24.0..=12.0).speed(0.1).fixed_decimals(1).suffix(" dB"));
        response.clone().on_hover_text("Song gain before deck EQ/FX and master. Applies to logical split fragments; saved with the mix. Independent of the deck fader.");
        if response.changed() {self.set_song_gain(i,db);}
        if ui.small_button("0 dB").on_hover_text("Reset this song’s gain").clicked() {self.set_song_gain(i,0.0);}
    }
    pub(super) fn request_source_levels(&mut self,i:usize) {
        let Some(t)=self.mix.tracks.get(i) else {return;};let path=t.path.clone();
        let Some((audio,_))=self.store.get(&path) else {self.status="Load the source before measuring levels.".into();return;};let audio=audio.clone();
        if self.source_levels.get(&path).is_some_and(|(a,_)|Arc::ptr_eq(a,&audio)) {return;}
        self.source_levels.insert(path.clone(),(audio.clone(),None));let tx=self.tx.clone();
        std::thread::spawn(move||{let result=crate::levels::SourceLevels::measure(&audio);let _=tx.send(Msg::SourceLevels(path,audio,result));});
    }
    pub(super) fn accept_source_levels(&mut self,path:String,audio:Arc<Audio>,result:Result<crate::levels::SourceLevels,String>) {
        if self.store.get(&path).is_some_and(|(current,_)|Arc::ptr_eq(current,&audio)) && self.source_levels.get(&path).is_some_and(|(current,_)|Arc::ptr_eq(current,&audio)) {self.source_levels.insert(path,(audio,Some(result)));}
    }
    pub(super) fn track_levels_ui(&mut self,ui:&mut egui::Ui,i:usize) {
        ui.horizontal_wrapped(|ui| {self.song_gain_control(ui,i,"Song gain");});
        let path=self.mix.tracks[i].path.clone();
        let measured=self.source_levels.get(&path).filter(|(a,_)|self.store.get(&path).is_some_and(|(b,_)|Arc::ptr_eq(a,b))).map(|(_,v)|v.clone());
        match measured {
            None=>{if ui.button("Measure whole-source peak / RMS").clicked() {self.request_source_levels(i);}},
            Some(None)=>{ui.label("Measuring source levels…");},
            Some(Some(Err(e)))=>{ui.colored_label(C::LIGHT_RED,e);},
            Some(Some(Ok(levels)))=>{
                ui.label(format!("Whole source · sample peak {} · stereo RMS {}",crate::levels::dbfs(levels.peak),crate::levels::dbfs(levels.rms)));
                let gain=crate::levels::amplitude(self.mix.tracks[i].gain_db);
                ui.label(format!("With song gain only · peak {} · RMS {}",crate::levels::dbfs(levels.peak*gain),crate::levels::dbfs(levels.rms*gain)));
                ui.label(egui::RichText::new("Includes all source silence. Source estimates exclude arrangement, stems, transitions, deck processing and master. RMS is not LUFS or perceived loudness.").color(DIM));
            }
        }
    }
    pub(super) fn set_master_levels(&mut self,gain:f32,output:crate::levels::OutputSettings) {
        if !gain.is_finite() || !(0.0..=4.0).contains(&gain) || !output.ceiling_db.is_finite() || !(-12.0..=0.0).contains(&output.ceiling_db) {return;}
        if gain==self.mix.master && output==self.mix.output {return;}
        self.snapshot();self.mix.master=gain;self.mix.output=output;
    }
    pub(super) fn master_levels_ui(&mut self,ui:&mut egui::Ui) {
        let mut gain=self.mix.master;let mut output=self.mix.output;
        ui.horizontal_wrapped(|ui| {
            let mut db=20.0*gain.max(0.001).log10();
            if ui.add(egui::Slider::new(&mut db,-60.0..=12.0).text("Master gain (dB)")).changed() {gain=crate::levels::amplitude(db);}
            if ui.button("0 dB").clicked() {gain=1.0;}
            if gain==0.0 {ui.label("Music muted");}
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut output.enabled,"Soft clipper enabled");
            ui.add_enabled(output.enabled,egui::DragValue::new(&mut output.ceiling_db).range(-12.0..=0.0).speed(0.1).fixed_decimals(1).suffix(" dBFS ceiling"));
        });
        self.set_master_levels(gain,output);
        if !output.enabled {ui.colored_label(C::LIGHT_YELLOW,"Bypass: output can exceed 0 dBFS and clip at the device or in encoded files.");}
        ui.label(egui::RichText::new("Sample-peak soft clipping, not a true-peak limiter. Master gain affects music; the click has independent volume and shares the output ceiling.").color(DIM));
        let pre=Shared::get(&self.out.shared.pre_peak);let post=Shared::get(&self.out.shared.post_peak);let ratio=Shared::get(&self.out.shared.min_ratio);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(if pre>1.0 {C::LIGHT_RED}else{TXT},format!("Before clipper · peak hold {}",crate::levels::dbfs(pre)));
            ui.colored_label(if post>1.0 {C::LIGHT_RED}else{TXT},format!("Output · peak hold {}",crate::levels::dbfs(post)));
            ui.label(format!("Peak soft-clip attenuation {:.1} dB",-20.0*ratio.max(1e-6).log10()));
            if ui.button("Reset peak holds").clicked() {let _=self.out.tx.send(Cmd::ResetLevels);}
        });
        ui.small("Reset starts with the next complete device block; paused holds clear on the next callback.");
        if pre>1.0 {ui.colored_label(C::LIGHT_YELLOW,"The summed input exceeded 0 dBFS. Lower song, deck or master gain if you want less clipping.");}
        ui.label(egui::RichText::new("Peak holds cover audio consumed by the device since reset. Playback meters and the metronome are excluded from export; gain and clipper settings are included.").color(DIM));
        ui.separator();
    }
}
