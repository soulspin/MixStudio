//! Sample drag/copy state; source clips remain untouched until a valid drop.
use super::*;
#[derive(Default)]
pub(super) struct State {
    pub selected:std::collections::BTreeSet<usize>,
    pub box_origin:Option<egui::Pos2>,
    pub originals:Vec<(usize,f64)>,
    pub copy:Option<CopyDrag>,
}
pub(super) struct CopyDrag {pub ti:usize,pub a:f64,pub b:f64,pub anchor:f64,pub stems:Option<Vec<u8>>}
impl App {
    pub(super) fn begin_sample_copy(&mut self,ti:usize,anchor:f64,lane:usize) {
        let t=&self.mix.tracks[ti];
        let (a,b,stems)=if let Some((_,l,a,b))=self.stem_sel.filter(|s|s.0==ti) {(a.min(b),a.max(b),Some(crate::model::lane_stems(t.stem_view,l)))}
        else if let Some((_,a,b))=self.section_range.filter(|s|s.0==ti) {(a,b,None)}
        else {(t.vis_start(0.0),t.end(0.0),if t.stem_view>0 {Some(crate::model::lane_stems(t.stem_view,lane))}else{None})};
        if stems.is_some()&&!self.stems.contains_key(&t.path){self.status="Separate stems before copying a stem".into();return;}
        self.samples.copy=Some(CopyDrag{ti,a,b,anchor,stems});self.status="Drop on Sample 1 or Sample 2. Escape cancels.".into();
    }
    pub(super) fn finish_sample_copy(&mut self,copy:CopyDrag,lane:usize,at:f64) {
        if self.arrangement_locked(){return;}
        // Build the result before recording undo, so invalid drops never dirty the project.
        let mut mix=self.mix.clone();
        match mix.sample_range(copy.ti,copy.a,copy.b,lane,at,copy.stems.as_deref()) {
            Ok(i)=>{self.snapshot();self.mix=mix;self.sel=Some(i);self.samples.selected.clear();self.samples.selected.insert(i);self.clear_section();self.status="Sample copied in sync. Drag to move; corner handles set fades.".into();}
            Err(e)=>self.status=e,
        }
    }
}
