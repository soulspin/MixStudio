//! Non-destructive sample extraction and atomic group movement.
use crate::model::Mix;
impl Mix {
    pub fn sample_range(&mut self,ti:usize,a:f64,b:f64,lane:usize,at:f64,stems:Option<&[u8]>)->Result<usize,String> {
        if self.has_transition_locks(){return Err("Unlock protected transitions before copying samples".into());}
        if !(2..=3).contains(&lane)|| !at.is_finite()||at<0.0 {return Err("Choose a sample lane and a nonnegative position".into());}
        let source=self.tracks.get(ti).ok_or("Select a source clip")?;let copy=source.copy_section(a,b)?;
        let ratio=crate::model::RATIOS.iter().position(|(_,m)|m.is_some_and(|m|(m-source.beat_mult).abs()<1e-8)).unwrap_or(1) as u8;
        let mut t=source.clone();t.set_sections(copy.spans);t.trans=Default::default();t.trans.ratio=ratio;t.locked=false;t.lane=lane;t.stem_view=0;
        t.start_beat+=at-t.vis_start(0.0);
        let relative=at-t.start_beat;t.auto=copy.auto.into_iter().map(|p|p.into_iter().map(|(b,v)|(b+relative,v)).collect()).collect();
        t.fade_in=0.0;t.fade_out=0.0;t.fade_curve=[0.0;2];
        if let Some(stems)=stems {for k in 0..4 {if !stems.contains(&k){t.set_mute(&[k],0.0,t.duration,true);}}}
        t.title=format!("{} (sample)",t.title);self.tracks.push(t);Ok(self.tracks.len()-1)
    }
    /// Clamp the group at mix beat zero without changing its internal spacing.
    pub fn shift_samples(&mut self,indices:&[usize],delta:f64)->Result<(),String> {
        if self.has_transition_locks()||!delta.is_finite(){return Err("Unlock editing and use a finite move".into());}
        let ids:std::collections::BTreeSet<_>=indices.iter().copied().collect();
        let mut earliest=f64::INFINITY;
        for &i in &ids {let t=self.tracks.get(i).ok_or("Sample no longer exists")?;if t.lane<2{return Err("Only sample clips can move as a sample group".into());}earliest=earliest.min(t.vis_start(0.0));}
        let delta=delta.max(-earliest);for i in ids {self.tracks[i].start_beat+=delta;}Ok(())
    }
}
