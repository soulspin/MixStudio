//! Non-destructive arrangement of source ranges within one song.

use crate::model::{Mix, Track, interp};

#[derive(Clone)]
pub struct SectionCopy {
    pub path: String,
    pub beats: f64,
    pub spans: Vec<(f64, f64)>,
    pub auto: Vec<Vec<(f64, f32)>>,
}

impl Track {
    pub fn set_sections(&mut self, spans: Vec<(f64,f64)>) {
        let previous=self.source_spans();
        if spans.first().map(|p|p.0)!=previous.first().map(|p|p.0) { self.trans.splice_fades[0]=false; }
        if spans.last().map(|p|p.1)!=previous.last().map(|p|p.1) { self.trans.splice_fades[1]=false; }
        let mut merged: Vec<(f64,f64)>=vec![];
        for (a,b) in spans { if let Some(last)=merged.last_mut() { if (last.1-a).abs()<1e-8 { last.1=b; continue; } } merged.push((a,b)); }
        if merged.len()==1 && (merged[0].0-self.trim_in).abs()<1e-8 && (merged[0].1-(self.duration-self.trim_out)).abs()<1e-8 { merged.clear(); }
        self.sections=merged;
    }
    pub fn source_spans(&self) -> Vec<(f64, f64)> {
        if self.sections.is_empty() { vec![(self.trim_in, self.duration-self.trim_out)] } else { self.sections.clone() }
    }
    /// Slice in arrangement beats without evaluating an endpoint in the next source span.
    pub fn section_spans(&self, a: f64, b: f64) -> Vec<(f64, f64)> {
        let g = self.source_grid(); let mut dest = self.vis_start(0.0); let mut out = vec![];
        for (s,e) in self.source_spans() {
            let u = g.beat(s); let len = g.beat(e)-u;
            let lo = a.max(dest); let hi = b.min(dest+len);
            if hi > lo+1e-9 { out.push((g.sec(u+lo-dest), g.sec(u+hi-dest))); }
            dest += len;
        } out
    }
    pub fn copy_section(&self, a: f64, b: f64) -> Result<SectionCopy, String> {
        if !a.is_finite() || !b.is_finite() || b-a < 0.01 || a < self.vis_start(0.0)-1e-7 || b > self.end(0.0)+1e-7 {
            return Err("Select a range inside one song".into());
        }
        let lo = a-self.start_beat; let hi = b-self.start_beat;
        let auto = self.auto.iter().map(|points| {
            if points.is_empty() { return vec![]; }
            let mut v = vec![(0.0, interp(points,lo).unwrap())];
            v.extend(points.iter().filter(|p| p.0>lo && p.0<hi).map(|p| (p.0-lo,p.1)));
            v.push((hi-lo,interp(points,hi).unwrap())); v
        }).collect();
        Ok(SectionCopy { path: self.path.clone(), beats: b-a, spans: self.section_spans(a,b), auto })
    }
}

fn changed_time(x: f64, a: f64, b: f64, delta: f64) -> f64 {
    if delta >= 0.0 { if x >= a { x+delta } else { x } }
    else if x >= b { x+delta } else if x >= a { a } else { x }
}

impl Mix {
    fn ripple_section(&mut self, ti: usize, a: f64, b: f64, delta: f64) {
        let order = self.main_order();
        let following: Vec<usize> = order.iter().position(|i| *i==ti).map(|k| order[k+1..].to_vec()).unwrap_or_default();
        for (i,t) in self.tracks.iter_mut().enumerate() {
            if i != ti {
                if following.contains(&i) { t.start_beat+=delta; }
                else if t.lane>=2 { let vis=t.vis_start(0.0); t.start_beat+=changed_time(vis,a,b,delta)-vis; }
            }
        }
        for f in &mut self.fx {
            let end = changed_time(f.start+f.len,a,b,delta);
            f.start = changed_time(f.start,a,b,delta); f.len = (end-f.start).max(0.0);
        }
        self.fx.retain(|f| f.len>1e-7);
        if !self.tempo_pts.is_empty() {
            let va=crate::model::bpm_on(&self.tempo_pts,self.bpm,a);
            let vb=crate::model::bpm_on(&self.tempo_pts,self.bpm,b);
            let mut points: Vec<_>=self.tempo_pts.iter().filter(|p| p.0<a || p.0>b).map(|p| (changed_time(p.0,a,b,delta),p.1)).collect();
            if delta>=0.0 { points.extend([(a,va),(a+delta,va)]); }
            else { points.extend([(a-1e-7,va),(a,vb)]); }
            points.sort_by(|a,b| a.0.total_cmp(&b.0)); self.tempo_pts=points;
        }
    }
    pub fn insert_section(&mut self, ti: usize, at: f64, copy: &SectionCopy) -> Result<f64,String> {
        if self.has_transition_locks() {return Err("Unlock protected transitions before editing sections".into());}
        let t = self.tracks.get(ti).ok_or("Select the source song")?;
        if t.path != copy.path { return Err("Paste into the same source song in this version".into()); }
        if !at.is_finite() || at<t.vis_start(0.0)-1e-7 || at>t.end(0.0)+1e-7 { return Err("Place the playhead inside the source song".into()); }
        let g = t.source_grid();
        if !copy.beats.is_finite() || copy.beats<=0.0 || copy.spans.is_empty() || copy.spans.iter().any(|(a,b)| !a.is_finite() || !b.is_finite() || *a<0.0 || *b>t.duration+1e-7 || b<=a) { return Err("Invalid source section".into()); }
        let len: f64 = copy.spans.iter().map(|(a,b)| g.beat(*b)-g.beat(*a)).sum();
        if !len.is_finite() || len<0.01 { return Err("Section is too short".into()); }
        let mut spans = t.section_spans(t.vis_start(0.0),at); spans.extend(copy.spans.iter().copied()); spans.extend(t.section_spans(at,t.end(0.0)));
        let p = at-t.start_beat;
        let mut auto = vec![];
        for k in 0..t.auto.len().max(copy.auto.len()) {
            let points = t.auto.get(k).cloned().unwrap_or_default();
            let insert = copy.auto.get(k).cloned().unwrap_or_default();
            let mut v: Vec<_> = points.iter().filter(|q| q.0<p).copied().collect();
            if let Some(value) = interp(&points,p) { v.push((p-1e-7,value)); }
            v.extend(insert.iter().map(|q| (p+(q.0*len/copy.beats).min(len-1e-7),q.1)));
            if let Some(value) = interp(&points,p) { v.push((p+len,value)); }
            v.extend(points.iter().filter(|q| q.0>p).map(|q| (q.0+len,q.1)));
            v.sort_by(|a,b| a.0.total_cmp(&b.0)); auto.push(v);
        }
        self.tracks[ti].set_sections(spans); self.tracks[ti].auto = auto;
        self.ripple_section(ti,at,at,len); Ok(len)
    }
    pub fn delete_section(&mut self, ti: usize, a: f64, b: f64) -> Result<(),String> {
        if self.has_transition_locks() {return Err("Unlock protected transitions before editing sections".into());}
        let t = self.tracks.get(ti).ok_or("Select a song")?; t.copy_section(a,b)?;
        if t.end(0.0)-t.vis_start(0.0)-(b-a)<0.01 { return Err("Keep at least part of the song; use Remove track to remove it".into()); }
        let mut spans = t.section_spans(t.vis_start(0.0),a); spans.extend(t.section_spans(b,t.end(0.0)));
        let (lo,hi) = (a-t.start_beat,b-t.start_beat);
        let auto = t.auto.iter().map(|points| {
            let mut v: Vec<_> = points.iter().filter(|p| p.0<lo).copied().collect();
            if let Some(value)=interp(points,lo) { v.push((lo-1e-7,value)); }
            if let Some(value)=interp(points,hi) { v.push((lo,value)); }
            v.extend(points.iter().filter(|p| p.0>hi).map(|p| (p.0-(b-a),p.1))); v
        }).collect();
        self.tracks[ti].set_sections(spans); self.tracks[ti].auto=auto; self.tracks[ti].clamp_fades();
        self.ripple_section(ti,a,b,a-b); Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Mix, Track};
    fn track() -> Track {
        serde_json::from_str(r#"{"path":"test.wav","title":"Test","artist":"Test","bpm":120.0,"offset":0.0,"key":"8A","duration":32.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap()
    }
    #[test]
    fn split_at_source_cut_preserves_the_rendered_splice_fade() {
        use std::sync::Arc;
        for mult in [1.0,2.0] {
            let mut m=Mix::default();m.bpm=120.0;let mut t=track();t.stretch=2;t.beat_mult=mult;t.trans.ratio=if mult==1.0 {1} else {2};
            t.set_sections(vec![(0.0,4.0),(12.0,20.0)]);m.tracks.push(t);
            let cut=8.0*mult;
            let audio=Arc::new(crate::analysis::Audio{sr:8000,l:vec![0.2;256000],r:vec![0.2;256000]});
            let before=crate::engine::render_offline(Arc::new(crate::engine::Snap::build(&m,&[Some(audio.clone())])),8000,cut-0.1,cut+0.1).0;
            let right=m.split(0,cut).unwrap();m.split(right,cut+4.0*mult).unwrap();m.split(0,4.0*mult).unwrap();m.sync_transition(0);
            assert!(!m.resolve_ratios());
            let restored:Mix=serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
            let audios=vec![Some(audio);restored.tracks.len()];
            let after=crate::engine::render_offline(Arc::new(crate::engine::Snap::build(&restored,&audios)),8000,cut-0.1,cut+0.1).0;
            let diff=before.iter().zip(after).map(|(a,b)|(a-b).abs()).fold(0.0f32,f32::max);
            assert!(diff<1e-6,"split changed source-cut fade by {diff} at ratio {mult}");
        }
    }
    #[test]
    fn repeat_then_delete_preserves_source_and_playlist_identity() {
        let mut m = Mix::default(); m.tracks.push(track());
        let copy = m.tracks[0].copy_section(8.0, 16.0).unwrap();
        m.insert_section(0, 16.0, &copy).unwrap();
        assert_eq!(m.tracks.len(), 1);
        assert!((m.tracks[0].end(120.0) - 72.0).abs() < 1e-8);
        for (b, s) in [(7.0, 3.5), (15.0, 7.5), (16.0, 4.0), (23.0, 7.5), (24.0, 8.0)] {
            assert!((m.tracks[0].sec_at(b) - s).abs() < 1e-8, "beat {b}");
            assert!((m.tracks[0].local_bpm(b) - 120.0).abs() < 1e-8);
        }
        m.delete_section(0, 16.0, 24.0).unwrap();
        for b in 0..64 { assert!((m.tracks[0].sec_at(b as f64) - b as f64 * 0.5).abs() < 1e-8); }
        let restored: Mix = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(restored.tracks[0].duration, 32.0);
        assert_eq!(restored.tracks[0].end(120.0), 64.0);
    }
    #[test]
    fn ripple_keeps_transition_length_and_moves_samples_and_tempo() {
        let mut m = Mix::default(); let a = track(); let mut b = track(); b.start_beat = 48.0; b.lane = 1;
        let mut sample = track(); sample.start_beat = 52.0; sample.lane = 2;
        m.tracks = vec![a, b, sample]; m.tempo_pts.push((56.0, 120.0));
        let before = m.transitions()[0]; let copy = m.tracks[0].copy_section(8.0, 16.0).unwrap();
        m.insert_section(0, 16.0, &copy).unwrap();
        let after = m.transitions()[0];
        assert_eq!((after.2, after.3), (before.2 + 8.0, before.3 + 8.0));
        assert_eq!(m.tracks[2].start_beat, 60.0); assert!(m.tempo_pts.iter().any(|p| p.0==64.0));
        assert!(m.delete_section(0, 0.0, m.tracks[0].end(0.0)).is_err());
        assert!(m.insert_section(0, 1000.0, &copy).is_err());
    }
    #[test]
    fn dynamic_grid_ratio_trim_and_repeated_edits_keep_the_source_mapping() {
        let mut m=Mix::default(); let mut t=track();
        t.offset=0.125; t.start_beat=12.0; t.trim_in=1.25; t.trim_out=2.0; t.beat_mult=2.0;
        t.map=Some(std::sync::Arc::new(crate::beatmap::BeatMap { bpm:120.0, beats:(0..100).map(|i| i as f64*0.5+0.005*(i as f64*0.1).sin()).collect() }));
        let original=t.clone(); let a=t.vis_start(0.0)+4.0; let b=a+8.0; m.tracks.push(t);
        for _ in 0..4 { let c=m.tracks[0].copy_section(a,b).unwrap(); m.insert_section(0,b,&c).unwrap(); }
        assert_eq!(m.tracks.len(),1);
        for i in 0..32 { let beat=b+i as f64+0.25; let expected=original.sec_at(a+(i%8) as f64+0.25); assert!((m.tracks[0].sec_at(beat)-expected).abs()<1e-8); }
        for i in 0..100 { assert!((235.0..245.0).contains(&m.tracks[0].local_bpm(a+i as f64*0.5))); }
        m.delete_section(0,b,b+32.0).unwrap();
        assert_eq!(m.tracks[0].sections.len(),0,"returning to the original arrangement removes unnecessary splices");
        assert!((m.tracks[0].end(0.0)-original.end(0.0)).abs()<1e-8);
    }
    #[test]
    fn edited_tracks_can_be_split_and_trimmed_without_revealing_deleted_audio() {
        let mut m=Mix::default(); m.tracks.push(track()); m.delete_section(0,8.0,16.0).unwrap();
        let right=m.split(0,12.0).unwrap();
        assert_eq!(m.tracks[0].end(0.0),12.0); assert_eq!(m.tracks[right].vis_start(0.0),12.0);
        assert_eq!(m.tracks[right].sec_at(12.0),10.0);
        m.set_edge(right,true,16.0); assert_eq!(m.tracks[right].vis_start(0.0),16.0); assert_eq!(m.tracks[right].sec_at(16.0),12.0);
    }
    #[test]
    fn manual_tempo_ripple_preserves_interpolation_outside_the_edit() {
        let mut m=Mix::default(); m.tempo_mode=2; m.tracks.push(track()); m.tempo_pts=vec![(0.0,100.0),(64.0,164.0)];
        let original=m.clone(); let c=m.tracks[0].copy_section(8.0,16.0).unwrap(); m.insert_section(0,32.0,&c).unwrap();
        assert!((m.bpm_at(16.0)-original.bpm_at(16.0)).abs()<1e-7);
        assert!((m.bpm_at(56.0)-original.bpm_at(48.0)).abs()<1e-7);
        m.delete_section(0,32.0,40.0).unwrap();
        for b in [16.0,32.0,48.0] { assert!((m.bpm_at(b)-original.bpm_at(b)).abs()<1e-5); }
    }
    #[test]
    fn automation_repeats_with_audio_and_preserves_boundary_values() {
        let mut m = Mix::default(); let mut t = track(); t.auto = vec![vec![(0.0,0.0),(8.0,0.8),(16.0,0.4),(64.0,1.0)]]; m.tracks.push(t);
        let c = m.tracks[0].copy_section(8.0,16.0).unwrap(); m.insert_section(0,16.0,&c).unwrap();
        assert!((crate::model::interp(&m.tracks[0].auto[0],20.0).unwrap()-0.6).abs()<1e-5);
        m.delete_section(0,16.0,24.0).unwrap();
        assert!((crate::model::interp(&m.tracks[0].auto[0],12.0).unwrap()-0.6).abs()<1e-5);
    }
}

#[cfg(test)]
mod local_audio_check {
    /// Optional integration check: source files stay on the user's machine, outside the repository.
    #[test]
    #[ignore = "requires MIXSTUDIO_SECTION_TEST_PROJECT pointing to a local saved mix"]
    fn render_saved_pair_after_section_edits() {
        use std::sync::Arc;
        let path=std::env::var("MIXSTUDIO_SECTION_TEST_PROJECT").expect("local mix path");
        let mut mix: crate::model::Mix=serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(mix.tracks.len()>=2); let first=mix.main_order()[0];
        let original_transition=mix.transitions()[0]; let start=mix.tracks[first].vis_start(0.0);
        let copy=mix.tracks[first].copy_section(start+32.0,start+48.0).unwrap();
        mix.insert_section(first,start+48.0,&copy).unwrap();
        mix.delete_section(first,start+80.0,start+88.0).unwrap();
        let transition=mix.transitions()[0];
        assert!((transition.2-original_transition.2-8.0).abs()<1e-7);
        assert!((transition.3-original_transition.3-8.0).abs()<1e-7);
        for (_,bpm) in mix.tempo_curve() { assert!((100.0..160.0).contains(&bpm),"unstable Auto tempo: {bpm}"); }
        let audio: Vec<_>=mix.tracks.iter().map(|t|Some(Arc::new(crate::analysis::load(std::path::Path::new(&t.path)).unwrap().0))).collect();
        let snap=Arc::new(crate::engine::Snap::build(&mix,&audio));
        let dir=std::env::temp_dir().join("mixstudio-section-real-pair"); std::fs::create_dir_all(&dir).unwrap();
        for (name,a,b) in [("repeat",start+44.0,start+68.0),("cut",start+76.0,start+88.0),("transition",transition.2,transition.3)] {
            let (l,r)=crate::engine::render_offline(snap.clone(),44100,a,b);
            assert!(!l.is_empty() && l.len()==r.len());
            assert!(l.iter().chain(&r).all(|v|v.is_finite() && v.abs()<=1.0));
            let rms=(l.iter().map(|v|v*v).sum::<f32>()/l.len() as f32).sqrt(); assert!(rms>0.01,"silent {name}");
            println!("{name}: {:.2}s, RMS {rms:.4}",l.len() as f64/44100.0);
            crate::engine::write_wav(&dir.join(format!("{name}.wav")),&l,&r,44100).unwrap();
        }
        std::fs::write(dir.join("edited.mixstudio.json"),serde_json::to_string_pretty(&mix).unwrap()).unwrap();
    }
}

#[cfg(test)]
mod sample_tests {
    use crate::model::{Mix,Track};
    fn track()->Track {serde_json::from_str(r#"{"path":"sample.wav","title":"Song","artist":"Test","bpm":120.0,"offset":0.0,"key":"8A","duration":32.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap()}
    #[test]
    fn sample_extraction_keeps_repeated_source_ranges_and_automation() {
        let mut t=track(); t.sections=vec![(0.0,4.0),(8.0,12.0),(0.0,4.0)];t.auto_set(0,0.0,1.0);t.auto_set(0,24.0,0.4);
        let mut m=Mix::default();m.tracks.push(t);let before=serde_json::to_string(&m.tracks[0]).unwrap();
        let i=m.sample_range(0,6.0,18.0,2,32.0,None).unwrap();let t=&m.tracks[i];
        assert!((t.vis_start(0.0)-32.0).abs()<1e-8);assert!((t.end(0.0)-44.0).abs()<1e-8);
        for b in [0.1,3.0,7.0,11.9] {assert!((t.sec_at(32.0+b)-m.tracks[0].sec_at(6.0+b)).abs()<1e-8);}
        assert!((t.auto_at(0,38.0).unwrap()-m.tracks[0].auto_at(0,12.0).unwrap()).abs()<1e-6);
        assert_eq!(serde_json::to_string(&m.tracks[0]).unwrap(),before);
    }
    #[test]
    fn moving_sample_group_preserves_spacing_and_rejects_songs_atomically() {
        let mut m=Mix::default();let mut a=track();a.lane=2;a.start_beat=4.0;let mut b=a.clone();b.start_beat=12.0;b.lane=3;m.tracks=vec![a,b,track()];
        m.shift_samples(&[0,1,0],-20.0).unwrap();assert_eq!(m.tracks[0].start_beat,0.0);assert_eq!(m.tracks[1].start_beat,8.0);
        let before=serde_json::to_string(&m).unwrap();assert!(m.shift_samples(&[0,2],4.0).is_err());assert_eq!(serde_json::to_string(&m).unwrap(),before);
    }
    #[test]
    fn extracted_samples_keep_resolved_ratios_after_the_next_frame() {
        for (ratio,mult) in [(1,1.0),(2,2.0),(3,0.5),(4,1.5),(5,2.0/3.0)] {
            let mut t=track();t.trans.ratio=ratio;t.beat_mult=mult;
            let mut m=Mix::default();m.bpm=120.0;m.tracks.push(t);
            let i=m.sample_range(0,8.0,16.0,2,32.0,None).unwrap();let positions:Vec<_>=[32.1,35.0,39.9].map(|b|m.tracks[i].sec_at(b)).into();
            m.resolve_ratios();let t=&m.tracks[i];assert_eq!(t.beat_mult,mult,"ratio changed after extraction");assert!((t.vis_start(0.0)-32.0).abs()<1e-8);assert!((t.end(0.0)-40.0).abs()<1e-8);
            for (b,sec) in [32.1,35.0,39.9].into_iter().zip(positions) {assert!((t.sec_at(b)-sec).abs()<1e-8);}
        }
    }

}
