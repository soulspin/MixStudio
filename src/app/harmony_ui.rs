//! A source-label advisory next to the existing processed transition audition.
use super::*;
impl App {
    pub(super) fn harmony_ui(&mut self,ui:&mut egui::Ui,outgoing:usize,incoming:usize) {
        let a=self.mix.tracks[outgoing].key.clone();let b=self.mix.tracks[incoming].key.clone();
        let members=self.transition_members(incoming).unwrap_or([vec![outgoing],vec![incoming]]);
        let varispeed=members.iter().flatten().any(|&i|{let t=&self.mix.tracks[i];t.stretch==2 || (t.stretch==0 && !self.mix.key_lock)});
        let relation=crate::harmony::Relation::between(&a,&b);
        let colour=match relation {crate::harmony::Relation::Same|crate::harmony::Relation::Neighbor|crate::harmony::Relation::Relative=>C::LIGHT_GREEN,crate::harmony::Relation::Different=>C::LIGHT_YELLOW,crate::harmony::Relation::Unknown=>DIM};
        ui.horizontal_wrapped(|ui| {
            ui.strong("Harmonic match");ui.colored_label(colour,format!("A {} / B {} · {}",a,b,relation.label()));
            let loaded=members.iter().flatten().all(|i|self.store.contains_key(&self.mix.tracks[*i].path));
            if ui.add_enabled(loaded,egui::Button::new("Listen together")).on_hover_text("Use the processed transition audition with both tracks. No key, EQ or arrangement edits.").clicked() {self.start_transition_audition(incoming,3,self.transition_repeat);}
        });
        ui.label(egui::RichText::new("Whole-track key labels; not measured harmony for this overlap. Check by listening.").color(DIM).size(11.0));
        if varispeed {ui.colored_label(C::LIGHT_YELLOW,"Pitch follows tempo on at least one track. Source-key labels may differ from the audible keys.");}
        ui.collapsing("Key details and mixing tips",|ui| {
            for (side,key) in [("A",&a),("B",&b)] {ui.label(format!("{side}: {key} · {}",crate::harmony::key_name(key).unwrap_or("unrecognized key")));}
            ui.label(relation.reason());ui.label(relation.tip());
            if let Some(keys)=crate::harmony::common_matches(&a) {ui.label(format!("Common source-key matches from A: {keys}"));}
            ui.small("Tonality, chords and vocals can change within a song. A compatible label is a suggestion, not a guarantee; EQ does not change the key.");
        });
    }
}
