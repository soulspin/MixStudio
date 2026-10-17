use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::beatmap::{BeatMap, Grid};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum Preset { #[default] Crossfade, BassSwap, HighToLow, LowToHigh, EchoOut }
impl Preset {
    pub const ALL: [Preset; 5] = [Preset::BassSwap, Preset::Crossfade, Preset::HighToLow, Preset::LowToHigh, Preset::EchoOut];
    pub fn name(self) -> &'static str {
        match self { Preset::Crossfade => "Crossfade", Preset::BassSwap => "Bass swap", Preset::HighToLow => "High to low", Preset::LowToHigh => "Low to high", Preset::EchoOut => "Echo out" }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum StemMode { None, Swap, #[default] Crossfade, Full }
impl StemMode { pub const ALL: [StemMode; 4] = [StemMode::None, StemMode::Swap, StemMode::Crossfade, StemMode::Full];
    pub fn name(self) -> &'static str { match self { StemMode::None => "None", StemMode::Swap => "Swap", StemMode::Crossfade => "Crossfade", StemMode::Full => "Full" } } }

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Transition {
    /// Protect transition settings and freeze arrangement placement until unlocked.
    #[serde(default)] pub locked:bool,
    pub preset: Preset, pub stems: [StemMode; 4],
    /// custom volume curves over the transition (progress 0..1, gain); empty = preset curve
    #[serde(default)] pub curve_out: Vec<(f32, f32)>, #[serde(default)] pub curve_in: Vec<(f32, f32)>,
    /// beat ratio of the incoming track (RATIOS): 0 = auto (half / double time when clearly closer)
    #[serde(default)] pub ratio: u8,
    /// Fragments of a neutral split share one logical transition envelope.
    #[serde(default)] pub fragment_group: u64,
    /// Original source-splice fades retained at the outer edges of split fragments.
    #[serde(default)] pub splice_fades: [bool;2] }

/// (name, mix beats per track beat; None = auto)
pub const RATIOS: [(&str, Option<f64>); 6] = [("Auto", None), ("1:1", Some(1.0)), ("1:2", Some(2.0)), ("2:1", Some(0.5)), ("2:3", Some(1.5)), ("3:2", Some(2.0 / 3.0))];
fn one64() -> f64 { 1.0 }
/// Auto ratio: x2 or /2 only when it is clearly closer to the reference tempo (in log distance).
pub fn auto_mult(bpm: f64, reference: f64) -> f64 {
    if bpm <= 0.0 || reference <= 0.0 { return 1.0; }
    let d = |m: f64| (bpm * m / reference).ln().abs();
    let best = [2.0, 0.5].into_iter().min_by(|a, b| d(*a).partial_cmp(&d(*b)).unwrap()).unwrap();
    if d(best) < d(1.0) - 0.15 { best } else { 1.0 }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Track {
    #[serde(default)] pub sections: Vec<(f64, f64)>,
    pub path: String, pub title: String, pub artist: String,
    pub bpm: f64, pub offset: f64, pub key: String, pub duration: f64,
    /// mix beat where the track's first downbeat lands
    pub start_beat: f64,
    /// 0,1 = Deck 1/2, 2,3 = Sample 1/2
    pub lane: usize,
    /// transition *into* this track (from the previous track on the main decks)
    pub trans: Transition,
    /// dynamic beat map from analysis (absent in projects saved before v0.3: linear grid)
    #[serde(default)] pub map: Option<Arc<BeatMap>>,
    /// Auto-order keeps a locked track at its playlist position
    #[serde(default)] pub locked: bool,
    /// seconds cut from the start / end of the file (split and trim keep the source offsets)
    #[serde(default)] pub trim_in: f64, #[serde(default)] pub trim_out: f64,
    /// fade-in / fade-out length in beats
    #[serde(default)] pub fade_in: f64, #[serde(default)] pub fade_out: f64,
    /// stem view on the timeline: 0 = full track, 1 = instrumental / vocals, 2 = four stems
    #[serde(default)] pub stem_view: u8,
    /// muted stem ranges
    #[serde(default)] pub stem_mutes: Vec<StemMute>,
    /// automation lanes (see AUTO_PARAMS): (beat relative to the downbeat, value)
    #[serde(default)] pub auto: Vec<Vec<(f64, f32)>>,
    /// stretch mode (STRETCH_MODES): 0 = mix default (key lock setting), 1 key lock, 2 vinyl, 3 beat slice
    #[serde(default)] pub stretch: u8,
    /// resolved beat ratio (mix beats per track beat), kept up to date by `Mix::resolve_ratios`
    #[serde(default = "one64")] pub beat_mult: f64,
    /// grid mode of this clip (GRID_MODES): 0 fixed, 1 dynamic, 2 flex, 3 imported, 4 manual
    #[serde(default = "one8")] pub grid_mode: u8,
    /// play this clip flat at its average tempo (Auto tempo); otherwise it keeps its groove
    #[serde(default)] pub stabilize: bool,
    /// flex beat map (analysis) and the beat grid imported from another DJ app
    #[serde(default)] pub flex: Option<Arc<BeatMap>>, #[serde(default)] pub imported: Option<Arc<BeatMap>>,
}
fn one8() -> u8 { 1 }
#[cfg(test)]
mod grid_regressions {
    use super::*;

    fn track(mode: u8) -> Track {
        let mut t: Track = serde_json::from_str(r#"{
            "path":"test.wav","title":"Test","artist":"Test","bpm":120.0,
            "offset":0.61,"key":"8A","duration":60.0,"start_beat":8.0,
            "lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}
        }"#).unwrap();
        let map = Arc::new(BeatMap { beats: (0..128).map(|i| 0.5 + i as f64 * 0.5).collect(), bpm: 120.0 });
        t.map = Some(map.clone()); t.flex = Some(map.clone()); t.imported = Some(map);
        t.grid_mode = mode;
        t
    }

    #[test]
    fn flex_and_imported_grids_honor_fine_offset_adjustments() {
        for mode in [2, 3] {
            let mut t = track(mode);
            let before = t.sec_at(16.0);
            t.offset += 0.001;
            assert!((t.sec_at(16.0) - before - 0.001).abs() < 1e-9, "mode {mode} ignored a 1 ms correction");
        }
    }

    #[test]
    fn editing_a_grid_preserves_its_fractional_downbeat_anchor() {
        for mode in [1, 2, 3] {
            let mut t = track(mode);
            let before = t.sec_at(16.0);
            t.edit_map(|m| m.shift(0.010));
            assert!((t.sec_at(16.0) - before - 0.010).abs() < 1e-9, "mode {mode} jumped to a different beat phase");
        }
    }

    #[test]
    fn selecting_an_analysed_grid_restores_its_tempo_after_manual_bpm() {
        for mode in [1, 2, 3] {
            let mut t = track(0);
            t.bpm = 121.0;
            t.set_grid_mode(mode);
            assert!(t.grid().dynamic(), "mode {mode} silently fell back to a fixed grid");
            assert!((t.sec_at(t.start_beat + 8.0) - t.sec_at(t.start_beat) - 4.0).abs() < 1e-9);
        }
    }

    #[test]
    fn reported_two_track_mix_has_stable_auto_tempo_with_dynamic_grids() {
        // Timing metadata from the reported failure; no source audio or personal paths.
        let mut mix: Mix = serde_json::from_str(include_str!("../tests/fixtures/unstable-flex-pair.json")).unwrap();
        let broken = mix.tempo_curve();
        assert!(broken.iter().any(|(_, bpm)| *bpm < 100.0 || *bpm > 160.0), "fixture must reproduce the severe Auto-tempo modulation");
        for track in &mut mix.tracks { track.set_grid_mode(1); }
        for (beat, bpm) in mix.tempo_curve() {
            assert!((122.0..=128.0).contains(&bpm), "unexpected {bpm:.2} BPM at beat {beat:.2}");
        }
        let restored: Mix = serde_json::from_str(&serde_json::to_string(&mix).unwrap()).unwrap();
        assert!(restored.tracks.iter().all(|t| t.grid_mode == 1 && t.grid().dynamic()));
    }
}
pub const GRID_MODES: [&str; 5] = ["Fixed", "Dynamic", "Flex", "Imported", "Manual"];
pub const GRID_HELP: [&str; 5] = ["Linear grid at the average BPM", "The analysed beat map (follows small tempo drift)", "Piecewise beat map that follows deliberate tempo jumps and ramps",
    "The beat grid from Rekordbox / Traktor (when the track came from an import)", "Your own edits (beatgrid editing switches to Manual)"];
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridScope { All, Single, Following }

impl Track {
    pub fn source_grid(&self) -> Grid { self.grid_raw().scaled(self.beat_mult) }
    pub fn grid(&self) -> Grid { self.source_grid().arranged(&self.sections, self.trim_in).outer_fades(self.vis_start(0.0),self.end(0.0),self.trans.splice_fades) }
    /// Visible phrase ranges in absolute mix beats, retaining the source phrase index.
    /// Source phrase beats are interpreted on the unscaled source grid.
    pub fn phrase_ranges(&self, phrases: &[(f64,u8)]) -> Vec<(usize,u8,f64,f64)> {
        let raw=self.grid_raw(); let source=self.source_grid(); let mut dest=self.vis_start(0.0); let mut out=vec![];
        for (a,b) in self.source_spans() {
            let u=source.beat(a); let v=source.beat(b);
            for (k,&(beat,kind)) in phrases.iter().enumerate() {
                let lo=source.beat(raw.sec(self.start_beat+beat)).max(u);
                let end=phrases.get(k+1).map(|p|raw.sec(self.start_beat+p.0)).unwrap_or(self.duration);
                let hi=source.beat(end).min(v);
                if hi>lo { out.push((k,kind,dest+lo-u,dest+hi-u)); }
            }
            dest+=v-u;
        }
        out
    }
    /// All visible occurrences of a source phrase boundary (deleted boundaries have none).
    pub fn phrase_occurrences(&self, beat: f64) -> Vec<f64> {
        let sec=self.grid_raw().sec(self.start_beat+beat);
        self.grid().occurrences(sec).into_iter().filter(|b|*b>=self.vis_start(0.0)-1e-8 && *b<self.end(0.0)-1e-8).collect()
    }
    /// grid without the beat ratio (the beatgrid editor works on track beats)
    pub fn grid_raw(&self) -> Grid {
        // Snap only when selecting a grid, never when evaluating it: fine corrections
        // and fractional downbeat anchors must be identical in the UI and audio engine.
        Grid::new(self.cur_map(), self.bpm, self.offset, self.start_beat)
    }
    /// beat map used by the grid mode (None = linear)
    pub fn cur_map(&self) -> Option<Arc<BeatMap>> { match self.grid_mode { 0 => None, 2 => self.flex.clone().or(self.map.clone()), 3 => self.imported.clone().or(self.map.clone()), _ => self.map.clone() } }
    /// Switch the grid mode; the downbeat stays where it is in the mix (flex / imported: their nearest beat / downbeat).
    pub fn set_grid_mode(&mut self, mode: u8) {
        if mode == 4 { self.ensure_map(); self.grid_mode = 4; return; }
        let anchor = self.sec_at(self.start_beat); self.grid_mode = mode.min(4);
        if let Some(m) = self.cur_map() {
            if !Grid::new(Some(m.clone()), self.bpm, self.offset, self.start_beat).dynamic() {
                self.bpm = m.bpm;
            }
        }
        let pick = |m: &Option<Arc<BeatMap>>, bar: bool| m.as_ref().filter(|m| m.beats.len() >= 8).map(|m| { let u = m.u_of(anchor); let k = if bar { (u / 4.0).round() * 4.0 } else { u.round() }; m.beats[k.clamp(0.0, (m.beats.len() - 1) as f64) as usize] });
        if let Some(o) = match mode { 2 => pick(&self.flex, false), 3 => pick(&self.imported, true), _ => None } { self.offset = o; }
        self.normalize_grid();
    }
    /// average tempo of the clip on the mix grid (beats per minute over its visible part)
    pub fn clip_avg(&self) -> f64 { let g = self.grid(); let (a, b) = (self.vis_start(0.0), self.end(0.0)); let s = if self.sections.is_empty() { g.sec(b) - g.sec(a) } else { self.sections.iter().map(|(a,b)| b-a).sum() }; if s > 1e-6 && b > a { (b - a) * 60.0 / s } else { self.eff_bpm() } }
    /// local tempo around mix beat `x` (mix BPM at which the clip plays at its natural speed)
    pub fn local_bpm(&self, x: f64) -> f64 { let g = self.grid(); let d = g.region_sec(x + 1.0, x) - g.region_sec(x - 1.0, x); if d > 1e-6 { 120.0 / d } else { self.eff_bpm() } }
    /// tempo of the track on the mix grid (track BPM x beat ratio)
    pub fn eff_bpm(&self) -> f64 { self.bpm * self.beat_mult }
    /// mix beat -> source seconds
    pub fn sec_at(&self, b: f64) -> f64 { self.grid().sec(b) }
    pub fn vis_start(&self, _mix_bpm: f64) -> f64 { self.source_grid().beat(self.trim_in) }
    pub fn end(&self, _mix_bpm: f64) -> f64 { if self.sections.is_empty() { self.source_grid().beat(self.duration - self.trim_out) } else { let g = self.source_grid(); self.vis_start(0.0) + self.sections.iter().map(|(a,b)| g.beat(*b)-g.beat(*a)).sum::<f64>() } }
    /// beats from the file start to the first downbeat
    pub fn intro_beats(&self) -> f64 { self.start_beat - self.grid().beat(0.0) }
    /// Make sure the track has an editable beat map that maps one track beat to one map entry.
    pub fn ensure_map(&mut self) {
        let g = self.grid_raw(); if matches!(self.grid_mode, 1 | 4) && g.dynamic() && g.ratio() == 1.0 { return; }
        let m = BeatMap::from_grid(&g, self.start_beat, self.duration, self.bpm);
        self.map = Some(Arc::new(m)); self.grid_mode = 4;
    }
    /// Map index of the track beat at mix beat `start_beat + k` (call `ensure_map` first).
    pub fn map_index(&self, k: i64) -> usize {
        let s = self.sec_at(self.start_beat + k as f64);
        self.map.as_ref().map(|m| m.u_of(s).round().max(0.0) as usize).unwrap_or(0).min(self.map.as_ref().map(|m| m.beats.len() - 1).unwrap_or(0))
    }
    /// Edit the beat map; the downbeat stays on the same beat, so the engine and the timeline follow.
    pub fn edit_map<R>(&mut self, f: impl FnOnce(&mut BeatMap) -> R) -> R {
        self.ensure_map(); self.grid_mode = 4;
        let mut m = (**self.map.as_ref().unwrap()).clone();
        let anchor = m.u_of(self.offset);
        let r = f(&mut m);
        self.offset = m.t_of(anchor); self.map = Some(Arc::new(m)); self.normalize_grid(); r
    }
    /// Explicit correction in source seconds. Arranged clips must keep their cut timing.
    pub fn adjust_grid(&mut self, scope: GridScope, beat: f64, seconds: f64) -> Result<f64, String> {
        if !self.sections.is_empty() { return Err("Correct the source grid before arranging sections (Undo restores the source clip).".into()); }
        if !beat.is_finite() || !seconds.is_finite() || seconds.abs() > 0.25 || self.bpm <= 0.0 {
            return Err("Invalid beatgrid correction.".into());
        }
        let target = self.start_beat + (beat - self.start_beat).round();
        if scope != GridScope::All && (target < self.vis_start(0.0) || target > self.end(0.0) || (self.beat_mult-1.0).abs()>1e-9) {
            return Err("Select a beat inside an uncut clip with a 1:1 beat ratio.".into());
        }
        if seconds == 0.0 { return Ok(0.0); }
        let before=self.sec_at(target);
        if scope == GridScope::All {
            if self.grid().dynamic() { self.edit_map(|m|m.shift(seconds)); }
            else { self.offset+=seconds; self.normalize_grid(); }
        } else {
            // Materialize displayed beat positions, including fractional analysis anchors.
            // Editing the nearest original map node would also move neighbouring displayed beats.
            let m=BeatMap::from_grid(&self.grid_raw(),self.start_beat,self.duration,self.bpm);
            self.map=Some(Arc::new(m)); self.grid_mode=4;
            let k=(target-self.start_beat).round() as i64;
            let index=self.map_index(k); let pivot=self.map_index(k-4);
            self.edit_map(|m|match scope {
                GridScope::Single=>{m.move_single(index,seconds);},
                GridScope::Following=>{m.stretch_following(pivot,index,seconds);},
                GridScope::All=>unreachable!(),
            });
        }
        Ok(self.sec_at(target)-before)
    }
    /// keep `offset` the first downbeat in the file (step whole bars), without moving the audio
    pub fn normalize_grid(&mut self) {
        for _ in 0..4096 { let p = self.grid().sec(self.start_beat - 4.0); if p >= 0.0 { self.offset = p; self.start_beat -= 4.0; } else { break; } }
        for _ in 0..4096 { if self.offset >= 0.0 { break; } self.offset = self.grid().sec(self.start_beat + 4.0); self.start_beat += 4.0; }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Deck { pub vol: f32, pub hi: f32, pub mid: f32, pub low: f32, pub filter: f32, pub mute: bool, pub solo: bool }
impl Default for Deck { fn default() -> Self { Deck { vol: 0.85, hi: 0.0, mid: 0.0, low: 0.0, filter: 0.0, mute: false, solo: false } } }

#[derive(Clone, Serialize, Deserialize)]
pub struct Mix { pub name: String, pub bpm: f64, pub tracks: Vec<Track>, pub decks: [Deck; 4], #[serde(default = "one")] pub master: f32,
    #[serde(default = "yes")] pub key_lock: bool, #[serde(default)] pub metronome: bool, #[serde(default = "half")] pub metro_vol: f32, #[serde(default)] pub auto_stems: bool,
    /// stem colours inside transitions: 0 normal, 1 dimmed, 2 inverted
    #[serde(default)] pub trans_stem_style: u8,
    /// effect blocks on the timeline
    #[serde(default)] pub fx: Vec<crate::fx::FxBlock>,
    /// tempo lane (TEMPO_MODES), manual nodes (beat, BPM) and ramp (start, end BPM)
    #[serde(default)] pub tempo_mode: u8, #[serde(default)] pub tempo_pts: Vec<(f64, f64)>, #[serde(default)] pub ramp: (f64, f64),
    /// hand-edited phrase labels per file (replaces the detected phrases)
    #[serde(default)] pub phrase_edit: std::collections::BTreeMap<String, Vec<(f64, u8)>>,
    /// cue points imported from other DJ apps per file: (source seconds, name)
    #[serde(default)] pub cue_marks: std::collections::BTreeMap<String, Vec<(f64, String)>> }
fn yes() -> bool { true }
fn half() -> f32 { 0.5 }
fn one() -> f32 { 1.0 }
impl Default for Mix { fn default() -> Self { Mix { name: "My mix".into(), bpm: 128.0, tracks: vec![], decks: [Deck::default(); 4], master: 1.0, key_lock: true, metronome: false, metro_vol: 0.5, auto_stems: false, trans_stem_style: 0, fx: vec![], tempo_mode: 0, tempo_pts: vec![], ramp: (0.0, 0.0), phrase_edit: Default::default(), cue_marks: Default::default() } } }

impl Mix {
    /// indices of main-deck tracks sorted by start
    pub fn main_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.tracks.len()).filter(|&i| self.tracks[i].lane < 2).collect();
        v.sort_by(|&a, &b| (self.tracks[a].start_beat, self.tracks[a].trim_in).partial_cmp(&(self.tracks[b].start_beat, self.tracks[b].trim_in)).unwrap());
        v
    }
    fn main_groups(&self) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>>=vec![];
        let mut order=self.main_order();
        order.sort_by(|&a,&b|self.tracks[a].vis_start(self.bpm).total_cmp(&self.tracks[b].vis_start(self.bpm)));
        for i in order {
            let t=&self.tracks[i];
            let group=groups.iter_mut().find(|g| {
                let first=&self.tracks[g[0]]; let last=&self.tracks[*g.last().unwrap()];
                t.trans.fragment_group!=0 && t.trans.fragment_group==first.trans.fragment_group && t.path==first.path && t.lane==first.lane && (last.end(self.bpm)-t.vis_start(self.bpm)).abs()<1e-7
            });
            if let Some(g)=group { g.push(i); } else { groups.push(vec![i]); }
        }
        groups
    }
    /// Alternate whole logical songs, preserving the deck shared by contiguous split fragments.
    pub fn alternate_main_lanes(&mut self) {
        for (lane,group) in self.main_groups().into_iter().enumerate() {
            for i in group { self.tracks[i].lane=lane%2; }
        }
    }
    /// Move the incoming logical song, optionally including later logical songs.
    pub fn has_transition_locks(&self)->bool {self.tracks.iter().any(|t|t.trans.locked)}
    pub fn shift_logical_transition(&mut self,incoming:usize,delta:f64,following:bool) {
        if self.has_transition_locks() {return;}
        let groups=self.main_groups();
        let Some(k)=groups.iter().position(|g|g.contains(&incoming)) else { return; };
        for group in &groups[k..if following { groups.len() } else { k+1 }] {
            for &i in group { self.tracks[i].start_beat+=delta; }
        }
    }
    /// (out idx, in idx, start beat, end beat); fragments retain the logical overlap interval.
    pub fn transitions(&self) -> Vec<(usize, usize, f64, f64)> {
        let groups=self.main_groups();
        let mut r=vec![];
        for w in groups.windows(2) {
            let (a,b)=(&w[0],&w[1]);
            let s=self.tracks[b[0]].vis_start(self.bpm).max(self.tracks[a[0]].vis_start(self.bpm));
            let e=self.tracks[*a.last().unwrap()].end(self.bpm).min(self.tracks[*b.last().unwrap()].end(self.bpm));
            if e>s+0.5 { for &x in a { for &y in b { r.push((x,y,s,e)); } } }
        }
        r
    }
    /// One entry per logical overlap for timeline controls and comparison previews.
    /// Audio snapshots use `transitions` to address every visible fragment.
    pub fn logical_transitions(&self) -> Vec<(usize,usize,f64,f64)> {
        let mut result:Vec<(usize,usize,f64,f64)>=vec![];
        let same=|a:usize,b:usize| { let (a,b)=(&self.tracks[a],&self.tracks[b]); a.trans.fragment_group!=0 && a.trans.fragment_group==b.trans.fragment_group && a.path==b.path && a.lane==b.lane };
        for transition in self.transitions() {
            if !result.iter().any(|p|p.2==transition.2 && p.3==transition.3 && (p.0==transition.0 || same(p.0,transition.0)) && (p.1==transition.1 || same(p.1,transition.1))) { result.push(transition); }
        }
        result
    }
    /// Propagate an edited fragment's transition settings to its logical siblings.
    pub fn sync_transition(&mut self,ti:usize) {
        let Some(track)=self.tracks.get(ti) else { return; };
        let group=track.trans.fragment_group; if group==0 { return; }
        let transition=track.trans.clone(); let path=track.path.clone();
        for t in &mut self.tracks { if t.trans.fragment_group==group && t.path==path { let fades=t.trans.splice_fades;t.trans=transition.clone();t.trans.splice_fades=fades; } }
    }
    /// phrases of a file: hand edits when present, otherwise the detected ones
    pub fn phrases<'a>(&'a self, path: &str, det: &'a [(f64, u8)]) -> &'a [(f64, u8)] { self.phrase_edit.get(path).map(|v| v.as_slice()).unwrap_or(det) }
    /// Like `relayout`, but when the previous track has an Outro phrase and the next one an Intro,
    /// the intro starts exactly on the outro (`ph[i]`: phrases of track i, may be empty).
    pub fn relayout_phrases(&mut self, order: &[usize], overlap_bars: f64, ph: &[Vec<(f64, u8)>]) -> usize {
        self.layout_resolved(order,overlap_bars,Some(ph))
    }
    fn align_phrases(&mut self,order:&[usize],ph:&[Vec<(f64,u8)>]) -> usize {
        let mut n=0;
        for k in 1..order.len() {
            let (p,i)=(order[k-1],order[k]);
            let outgoing=ph.get(p).and_then(|v|v.last()).filter(|q|q.1==4).and_then(|q|self.tracks[p].phrase_occurrences(q.0).last().copied());
            let incoming=ph.get(i).and_then(|v|v.first()).filter(|p|p.1==0).and_then(|p|self.tracks[i].phrase_occurrences(p.0).first().copied());
            if let (Some(a),Some(b))=(outgoing,incoming) {
                for &j in &order[k..] { self.tracks[j].start_beat+=a-b; } n+=1;
            }
        }
        n
    }

    pub fn length_beats(&self) -> f64 { self.tracks.iter().map(|t| t.end(self.bpm)).fold(0.0, f64::max) }
    /// lay main tracks out in current order with `overlap_bars` overlap, alternating decks
    pub fn relayout(&mut self, order: &[usize], overlap_bars: f64) {
        self.layout_resolved(order,overlap_bars,None);
    }
    fn layout_resolved(&mut self, order:&[usize],overlap_bars:f64,phrases:Option<&[Vec<(f64,u8)>]>) -> usize {
        if self.has_transition_locks() {return 0;}
        let mut aligned;
        self.resolve_ratios_in(order);
        // Ratios are discrete; resample after placement until geometry and tempo agree.
        // A repeated ratio state means a threshold cycle, handled by stable explicit ratios.
        let mut seen=vec![];
        loop {
            let state:Vec<_>=self.tracks.iter().map(|t|t.beat_mult).collect();
            if let Some(cycle)=seen.iter().position(|old|old==&state) {
                for (i,t) in self.tracks.iter_mut().enumerate() {
                    if t.trans.ratio==0 && seen[cycle..].iter().any(|old:&Vec<f64>|old[i]!=state[i]) {
                        t.trans.ratio=RATIOS.iter().position(|r|r.1==Some(t.beat_mult)).unwrap_or(1) as u8;
                    }
                }
            }
            seen.push(state);
            let mut cursor = 0.0f64;
            for (k, &i) in order.iter().enumerate() {
                let bpm = self.bpm;
                let t = &mut self.tracks[i];
                t.lane = k % 2;
                if k == 0 { t.start_beat = (t.intro_beats() / 4.0 - 0.01).ceil() * 4.0; }
                else { let intro = t.intro_beats(); t.start_beat = ((cursor - overlap_bars * 4.0 + intro) / 4.0).round() * 4.0; }
                cursor = t.end(bpm);
            }
            aligned=phrases.map(|ph|self.align_phrases(order,ph)).unwrap_or(0);
            if !self.resolve_ratios_in(order) { break; }
        }
        aligned
    }

    pub fn add_track(&mut self, mut t: Track) {
        if self.tracks.iter().all(|x| x.lane >= 2) { self.bpm = t.bpm.round(); }
        let o = self.main_order();
        if let Some(&last) = o.last() {
            let p = &self.tracks[last];
            t.lane = 1 - p.lane;
            let intro = t.intro_beats();
            t.start_beat = ((p.end(self.bpm) - 64.0 + intro) / 4.0).round() * 4.0;
            t.start_beat = t.start_beat.max(p.start_beat + 4.0);
        } else { t.lane = 0; t.start_beat = (t.intro_beats() / 4.0 - 0.01).ceil() * 4.0; }
        self.tracks.push(t);
    }
}

/// Camelot distance (0 = same, 1 = adjacent/relative)
#[allow(dead_code)]
pub fn camelot_dist(a: &str, b: &str) -> i32 {
    let p = crate::automix::camelot;
    match (p(a), p(b)) {
        (Some((na, la)), Some((nb, lb))) => { let d = (na - nb).rem_euclid(12); let d = d.min(12 - d); d + if la != lb { 1 } else { 0 } }
        _ => 6,
    }
}

/// Magnetic snapping: the candidate closest to `b` within `tol` beats.
pub fn magnet(b: f64, cands: &[f64], tol: f64) -> Option<f64> {
    cands.iter().copied().filter(|c| (c - b).abs() <= tol).min_by(|x, y| (x - b).abs().partial_cmp(&(y - b).abs()).unwrap())
}

impl Mix {
    /// Split track `ti` at mix beat `b`. Both clips keep the source offsets; returns the new clip index.
    pub fn split(&mut self, ti: usize, b: f64) -> Option<usize> {
        if self.has_transition_locks() {return None;}
        let t = self.tracks.get(ti)?;
        if b-t.vis_start(0.0)<0.1 || t.end(0.0)-b<0.1 { return None; }
        if t.sections.is_empty() { let s=t.sec_at(b); if s<=t.trim_in+0.05 || s>=t.duration-t.trim_out-0.05 { return None; } }
        let splice=t.grid().cuts().any(|cut|(cut-b).abs()<1e-8);
        let group=if t.trans.fragment_group!=0 { t.trans.fragment_group } else { self.tracks.iter().map(|t|t.trans.fragment_group).max().unwrap_or(0).saturating_add(1) };
        self.tracks[ti].trans.fragment_group=group;
        let t=&self.tracks[ti];
        if !t.sections.is_empty() {
            let (vs,ve)=(t.vis_start(0.0),t.end(0.0)); if b-vs<0.1 || ve-b<0.1 { return None; }
            let mut right=t.clone(); right.set_sections(t.section_spans(b,ve)); right.start_beat+=b-vs;
            for points in &mut right.auto { for p in points { p.0-=b-vs; } }
            right.fade_in=0.0; right.trans.splice_fades[0]=splice; right.locked=false;
            let spans=t.section_spans(vs,b); self.tracks[ti].set_sections(spans); self.tracks[ti].fade_out=0.0; self.tracks[ti].trans.splice_fades[1]=splice;
            self.tracks.push(right); return Some(self.tracks.len()-1);
        }
        let s = t.sec_at(b);
        if s <= t.trim_in + 0.05 || s >= t.duration - t.trim_out - 0.05 { return None; }
        let mut right = t.clone();
        right.trim_in = s; right.fade_in = 0.0; right.trans.splice_fades[0]=splice; right.locked = false;
        let left = &mut self.tracks[ti]; left.trim_out = left.duration - s; left.fade_out = 0.0; left.trans.splice_fades[1]=splice;
        self.tracks.push(right); Some(self.tracks.len() - 1)
    }
    /// Move the left or right edge of a clip to mix beat `b` (trim or extend, at least one beat long).
    pub fn set_edge(&mut self, ti: usize, left: bool, b: f64) {
        if self.has_transition_locks() {return;}
        let t = &mut self.tracks[ti];
        let edge=if left { t.vis_start(0.0) } else { t.end(0.0) };
        if (edge-b).abs()>1e-8 { t.trans.splice_fades[if left {0} else {1}]=false; }
        if !t.sections.is_empty() {
            let (vs,ve)=(t.vis_start(0.0),t.end(0.0));
            if left {
                let b=b.clamp(vs,(ve-0.1).max(vs)); t.set_sections(t.section_spans(b,ve)); t.start_beat+=b-vs;
                for points in &mut t.auto { for p in points { p.0-=b-vs; } }
            } else { let b=b.clamp((vs+0.1).min(ve),ve); t.set_sections(t.section_spans(vs,b)); }
            t.fade_in=t.fade_in.min(t.end(0.0)-t.vis_start(0.0)); t.fade_out=t.fade_out.min(t.end(0.0)-t.vis_start(0.0)); return;
        }
        let s = t.sec_at(b); let min = 60.0 / t.bpm;
        if left { t.trim_in = s.clamp(0.0, (t.duration - t.trim_out - min).max(0.0)); t.fade_in = t.fade_in.min(t.end(0.0) - t.vis_start(0.0)); }
        else { t.trim_out = (t.duration - s).clamp(0.0, (t.duration - t.trim_in - min).max(0.0)); t.fade_out = t.fade_out.min(t.end(0.0) - t.vis_start(0.0)); }
    }
    /// Copy a clip (Alt+drag on a sample); returns the copy's index.
    pub fn duplicate(&mut self, ti: usize) -> usize { let mut c = self.tracks[ti].clone(); c.trans.fragment_group=0; c.trans.locked=false; c.locked = false; self.tracks.push(c); self.tracks.len() - 1 }
    /// Put a clip on `lane` (0/1 = decks, 2/3 = sample lanes); deck clips alternate Deck 1 / Deck 2 in time order.
    pub fn move_to_lane(&mut self, ti: usize, lane: usize) {
        if self.has_transition_locks() {return;}
        let previous=self.tracks[ti].lane;
        let members=if lane<2 && previous<2 { self.main_groups().into_iter().find(|g|g.contains(&ti)).unwrap_or_else(||vec![ti]) } else {vec![ti]};
        for i in members { self.tracks[i].lane=lane.min(3); }
        if lane>=2 && previous<2 { self.tracks[ti].trans.fragment_group=0; }
        if lane < 2 && self.tracks[ti].trans.fragment_group==0 { self.tracks[ti].trans = Default::default(); }
        self.alternate_main_lanes();
    }
}

/// A muted range (source seconds) on one stem: 0 drums, 1 bass, 2 melody, 3 vocals.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct StemMute { pub stem: u8, pub a: f64, pub b: f64 }

/// Stems shown in each lane of a 2- or 4-lane stem view.
pub fn lane_stems(view: u8, lane: usize) -> Vec<u8> { if view == 1 { if lane == 0 { vec![0, 1, 2] } else { vec![3] } } else { vec![lane.min(3) as u8] } }
pub fn lane_name(view: u8, lane: usize) -> &'static str { if view == 1 { ["Instrumental", "Vocals"][lane.min(1)] } else { ["Drums", "Bass", "Melody", "Vocals"][lane.min(3)] } }

impl Track {
    /// Mute (or unmute) `stems` between source seconds `a` and `b`; ranges merge and split as needed.
    pub fn set_mute(&mut self, stems: &[u8], a: f64, b: f64, mute: bool) {
        let (a, b) = (a.min(b), a.max(b));
        for &k in stems {
            let (mut na, mut nb) = (a, b); let mut out = vec![];
            for m in self.stem_mutes.drain(..) {
                if m.stem != k || m.b < a || m.a > b { out.push(m); continue; }
                if mute { na = na.min(m.a); nb = nb.max(m.b); }
                else { if m.a < a { out.push(StemMute { stem: k, a: m.a, b: a }); } if m.b > b { out.push(StemMute { stem: k, a: b, b: m.b }); } }
            }
            if mute { out.push(StemMute { stem: k, a: na, b: nb }); }
            self.stem_mutes = out;
        }
    }
    pub fn muted(&self, stem: u8, sec: f64) -> bool { self.stem_mutes.iter().any(|m| m.stem == stem && sec >= m.a && sec < m.b) }
    /// A sample-lane clip of this track's audio between source seconds `a`..`b`, rendered to `path`,
    /// placed so it plays in sync with the original.
    pub fn clip_track(&self, a: f64, b: f64, path: String, title: String, lane: usize) -> Track {
        let mut t = self.clone(); t.sections.clear();
        t.path = path; t.title = title; t.lane = lane; t.duration = b - a; t.offset -= a;
        t.trim_in = 0.0; t.trim_out = 0.0; t.fade_in = 0.0; t.fade_out = 0.0; t.stem_mutes.clear(); t.stem_view = 0; t.locked = false; t.trans = Default::default();
        t.map = self.map.as_ref().map(|m| { let mut m = (**m).clone(); m.shift(-a); Arc::new(m) });
        t.normalize_grid(); t
    }
}

/// Automation parameters per track: volume (0..1), low / mid / high EQ and filter (-1..1).
pub const AUTO_PARAMS: [&str; 5] = ["Volume", "Low", "Mid", "High", "Filter"];
/// Linear interpolation through automation points (x sorted); None when the lane is empty.
pub fn interp(p: &[(f64, f32)], x: f64) -> Option<f32> {
    let (f, l) = (p.first()?, p.last()?);
    if x <= f.0 { return Some(f.1); } if x >= l.0 { return Some(l.1); }
    let i = p.partition_point(|q| q.0 <= x); let (a, b) = (p[i - 1], p[i]);
    Some(a.1 + (b.1 - a.1) * ((x - a.0) / (b.0 - a.0).max(1e-9)) as f32)
}
pub fn interp32(p: &[(f32, f32)], x: f32) -> f32 {
    let v: Vec<(f64, f32)> = p.iter().map(|q| (q.0 as f64, q.1)).collect(); interp(&v, x as f64).unwrap_or(1.0)
}
impl Track {
    /// Automation value of parameter `k` at mix beat `b` (points are stored relative to the downbeat).
    #[allow(dead_code)]
    pub fn auto_at(&self, k: usize, b: f64) -> Option<f32> { self.auto.get(k).and_then(|p| interp(p, b - self.start_beat)) }
    /// Insert or replace a point (track-relative beat x) keeping the lane sorted; returns its index.
    pub fn auto_set(&mut self, k: usize, x: f64, v: f32) -> usize {
        while self.auto.len() < AUTO_PARAMS.len() { self.auto.push(vec![]); }
        let p = &mut self.auto[k];
        p.retain(|q| (q.0-x).abs() >= 1e-6);
        let i = p.partition_point(|q| q.0 < x); p.insert(i, (x, v)); i
    }
}

pub const TEMPO_MODES: [&str; 4] = ["Constant", "Auto", "Manual", "Fixed ramp"];
pub const STRETCH_MODES: [&str; 4] = ["Mix default", "Key lock (WSOLA)", "Vinyl (varispeed)", "Beat slice"];
impl Mix {
    /// Tempo curve as (mix beat, BPM) points, linear in between; empty = constant `bpm`.
    pub fn tempo_curve(&self) -> Vec<(f64, f64)> {
        match self.tempo_mode {
            1 => { // Auto: each clip at its own tempo (its groove, or flat when stabilized), gliding through the transitions
                let groups=self.main_groups(); let tr=self.transitions(); let mut v:Vec<(f64,f64)>=vec![];
                for group in groups {
                    let first=&self.tracks[group[0]];
                    let (mut logical_a,mut logical_b)=(first.vis_start(self.bpm),self.tracks[*group.last().unwrap()].end(self.bpm));
                    for &(x,y,s,e) in &tr { if group.contains(&y) { logical_a=logical_a.max(e); } if group.contains(&x) { logical_b=logical_b.min(s); } }
                    if logical_b<logical_a { let c=0.5*(logical_a+logical_b);logical_a=c;logical_b=c; }
                    // A neutral split retains one sampling cadence and stabilized average.
                    // Deliberate per-fragment tempo policies instead form separate runs.
                    let mut runs:Vec<Vec<usize>>=vec![];
                    for i in group {
                        let t=&self.tracks[i];
                        if let Some(run)=runs.last_mut() {
                            let previous=&self.tracks[*run.last().unwrap()];
                            if previous.stabilize==t.stabilize && previous.grid_mode==t.grid_mode {run.push(i);continue;}
                        }
                        runs.push(vec![i]);
                    }
                    for run in runs {
                        let t=&self.tracks[run[0]];
                        let a=logical_a.max(t.vis_start(self.bpm));let b=logical_b.min(self.tracks[*run.last().unwrap()].end(self.bpm));
                        if b<a {continue;}
                        if t.stabilize || !t.grid().dynamic() {
                            let beats:f64=run.iter().map(|&i|self.tracks[i].end(0.0)-self.tracks[i].vis_start(0.0)).sum();
                            let seconds:f64=run.iter().map(|&i| { let t=&self.tracks[i]; (t.end(0.0)-t.vis_start(0.0))*60.0/t.clip_avg() }).sum();
                            let avg=if seconds>1e-6 { beats*60.0/seconds } else { t.eff_bpm() };
                            let e=if b-a>1e-3 { 1e-4 } else { 0.0 };v.push((a+e,avg));v.push((b-e,avg));
                        } else {
                            let mut x=a;
                            loop {
                                let i=run.iter().copied().find(|&i|x<self.tracks[i].end(0.0)).unwrap_or(*run.last().unwrap());
                                v.push((x,self.tracks[i].local_bpm(x)));if x>=b { break; }x=(x+2.0).min(b);
                            }
                        }
                    }
                }
                v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap()); v.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9); v
            }
            2 => { let mut v = self.tempo_pts.clone(); v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap()); v }
            3 => { let (s, e) = (if self.ramp.0 > 0.0 { self.ramp.0 } else { self.bpm }, if self.ramp.1 > 0.0 { self.ramp.1 } else { self.bpm }); vec![(0.0, s), (self.length_beats().max(1.0), e)] }
            _ => vec![],
        }
    }
    /// Resolve every track's beat ratio (explicit, or auto against the previous track in Auto tempo,
    /// otherwise against the mix tempo where the track starts). The downbeat stays on its mix beat.
    pub fn resolve_ratios(&mut self) -> bool {
        self.resolve_ratios_in(&self.main_groups().into_iter().flatten().collect::<Vec<_>>())
    }
    fn resolve_ratios_in(&mut self, order: &[usize]) -> bool {
        if self.has_transition_locks() {return false;}
        let fixed_curve = if self.tempo_mode == 1 { vec![] } else { self.tempo_curve() };
        let mut prev: Option<f64> = None; let mut res = vec![]; let mut groups=std::collections::BTreeMap::new();
        let logical_groups=self.main_groups();
        // Arranged fragments have distinct downbeats. Scale their relative placements with
        // the shared ratio, retaining the first downbeat and source-trim mapping.
        let placements:Vec<_>=logical_groups.iter().filter(|g|g.len()>1).map(|g| {
            let first=g[0];(first,self.tracks[first].start_beat,self.tracks[first].beat_mult,g.iter().map(|&i|(i,self.tracks[i].start_beat)).collect::<Vec<_>>())
        }).collect();
        let mut membership=vec![usize::MAX;self.tracks.len()];
        for (id,group) in logical_groups.into_iter().enumerate() { for i in group { membership[i]=id; } }
        for &i in order { let t = &self.tracks[i];
            if let Some(&m)=groups.get(&membership[i]) { res.push((i,m));continue; }
            let m = match RATIOS[(t.trans.ratio as usize).min(RATIOS.len() - 1)].1 { Some(m) => m,
                None => if self.tempo_mode == 1 { prev.map(|p| auto_mult(t.bpm, p)).unwrap_or(1.0) } else { auto_mult(t.bpm, bpm_on(&fixed_curve, self.bpm, t.start_beat)) } };
            groups.insert(membership[i],m);
            prev = Some(t.bpm * m); res.push((i, m)); }
        for (i, t) in self.tracks.iter().enumerate() { if t.lane >= 2 {
            let reference = if self.tempo_mode == 1 { None } else { Some(bpm_on(&fixed_curve, self.bpm, t.start_beat)) };
            let m = match RATIOS[(t.trans.ratio as usize).min(RATIOS.len() - 1)].1 { Some(m) => m, None => reference.map(|r| auto_mult(t.bpm, r)).unwrap_or(1.0) }; res.push((i, m)); } }
        let mut changed = false;
        for (i, m) in res { if self.tracks[i].beat_mult != m { self.tracks[i].beat_mult = m; changed = true; } }
        for (first,anchor,old_ratio,members) in placements {
            let scale=self.tracks[first].beat_mult/old_ratio;
            if scale!=1.0 { for (i,start) in members { self.tracks[i].start_beat=anchor+(start-anchor)*scale; } }
        }
        changed
    }
    pub fn bpm_at(&self, b: f64) -> f64 { bpm_on(&self.tempo_curve(), self.bpm, b) }
    /// Seconds between two mix beats along the tempo curve.
    pub fn secs(&self, a: f64, b: f64) -> f64 { secs_on(&self.tempo_curve(), self.bpm, a, b) }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    fn track(bpm: f64, start: f64) -> Track {
        serde_json::from_value(serde_json::json!({"path":"test.wav","title":"Test","artist":"Test","bpm":bpm,"offset":0.0,"key":"8A","duration":120.0,"start_beat":start,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}})).unwrap()
    }
    #[test]
    fn grouped_ratio_changes_keep_arranged_and_trimmed_fragments_contiguous() {
        for explicit in [false,true] { for arranged in [false,true] {
            let mut m=Mix::default();m.bpm=120.0;
            let mut t=track(120.0,11.0);t.offset=0.25;t.trim_in=1.0;t.duration=16.0;t.trim_out=1.0;
            if arranged {t.set_sections(vec![(1.0,4.0),(8.0,12.0)]);}
            m.tracks.push(t);let mut unsplit=m.clone();let split=m.tracks[0].vis_start(0.0)+6.0;let right=m.split(0,split).unwrap();
            if explicit {m.tracks[0].trans.ratio=2;m.sync_transition(0);unsplit.tracks[0].trans.ratio=2;}
            else {m.bpm=240.0;unsplit.bpm=240.0;}
            assert!(m.resolve_ratios());unsplit.resolve_ratios();
            assert_eq!(m.tracks[0].start_beat,11.0);
            assert!((m.tracks[0].end(0.0)-m.tracks[right].vis_start(0.0)).abs()<1e-9,"explicit {explicit}, arranged {arranged}: fragments overlap or leave a gap");
            assert!((m.tracks[right].end(0.0)-unsplit.tracks[0].end(0.0)).abs()<1e-9);
            let start=unsplit.tracks[0].vis_start(0.0);let end=unsplit.tracks[0].end(0.0);
            for k in 0..32 {let beat=start+(end-start)*(k as f64+0.5)/32.0;let i=if beat<m.tracks[0].end(0.0) {0} else {right};assert!((m.tracks[i].sec_at(beat)-unsplit.tracks[0].sec_at(beat)).abs()<1e-9);}
            assert!(!m.resolve_ratios());
        }}
    }
    #[test]
    fn auto_ratio_reference_advances_once_per_logical_song() {
        let mut m=Mix::default();m.tempo_mode=1;
        let mut a=track(100.0,0.0);a.duration=300.0;a.set_sections(vec![(0.0,100.0),(110.0,290.0)]);
        let mut b=track(130.0,160.0);b.duration=300.0;b.lane=1;
        let mut c=track(75.0,300.0);c.duration=300.0;
        m.tracks=vec![a,b,c];m.resolve_ratios();assert_eq!(m.tracks[2].beat_mult,2.0);
        m.split(0,250.0).unwrap();assert!(!m.resolve_ratios());assert_eq!(m.tracks[2].beat_mult,2.0);
    }
    #[test]
    fn unrelated_lane_edit_preserves_split_sibling_decks_and_overlap() {
        let mut m=Mix::default();m.tempo_mode=1;m.bpm=120.0;
        let mut b=track(120.0,160.0);b.lane=1;let mut sample=track(120.0,500.0);sample.lane=2;
        m.tracks=vec![track(120.0,0.0),b,sample];let right=m.split(1,200.0).unwrap();
        m.move_to_lane(2,3);
        assert_eq!(m.tracks[1].lane,m.tracks[right].lane);
        m.move_to_lane(right,0);
        assert_eq!(m.tracks[1].lane,m.tracks[right].lane,"moving a split deck fragment keeps its logical siblings together");
        assert_eq!(m.logical_transitions().iter().map(|p|(p.2,p.3)).collect::<Vec<_>>(),vec![(160.0,240.0)]);
        assert!(!m.resolve_ratios());
    }
    #[test]
    fn transition_motion_moves_logical_groups_without_outgoing_fragments() {
        for following in [false,true] {
            let mut m=Mix::default();m.bpm=120.0;let mut b=track(120.0,160.0);b.lane=1;
            m.tracks=vec![track(120.0,0.0),b,track(120.0,360.0)];
            let incoming_right=m.split(1,200.0).unwrap();let outgoing_right=m.split(0,190.0).unwrap();
            let before:Vec<_>=m.tracks.iter().map(|t|t.start_beat).collect();
            m.shift_logical_transition(1,8.0,following);
            for i in [1,incoming_right] { assert_eq!(m.tracks[i].start_beat,before[i]+8.0); }
            for i in [0,outgoing_right] { assert_eq!(m.tracks[i].start_beat,before[i]); }
            assert_eq!(m.tracks[2].start_beat,before[2]+if following {8.0} else {0.0});
        }
    }
    #[test]
    fn exact_seconds_respect_tempo_floor_crossings() {
        let expected=16.0+32.0*2.0_f64.ln();
        for curve in [vec![(0.0,10.0),(16.0,40.0)],vec![(0.0,40.0),(16.0,10.0)]] {
            assert!((secs_on(&curve,120.0,0.0,16.0)-expected).abs()<1e-12);
            assert!((secs_on(&curve,120.0,16.0,0.0)+expected).abs()<1e-12);
        }
        assert_eq!(secs_on(&[(0.0,5.0),(16.0,15.0)],120.0,0.0,16.0),48.0);
    }
    #[test]
    fn reordered_auto_tempo_layout_survives_next_frame() {
        let mut m=Mix::default(); m.tempo_mode=1;
        m.tracks=vec![track(70.0,0.0),track(140.0,200.0),track(140.0,400.0)];
        m.resolve_ratios(); m.relayout(&[1,0,2],16.0);
        let before=m.transitions();
        assert!(!m.resolve_ratios(), "layout used ratios from the old order");
        assert_eq!(m.transitions(),before);
        assert!(before.iter().all(|x| (x.3-x.2-64.0).abs()<0.01));
    }
    #[test]
    fn split_overlap_retains_entire_envelope_on_both_sides() {
        for arranged in [false,true] {
            let mut m=Mix::default(); m.bpm=120.0; m.tempo_mode=1;
            let mut a=track(120.0,0.0); let mut b=track(120.0,160.0); b.path="incoming.wav".into();
            b.trans.preset=Preset::BassSwap; b.trans.curve_in=vec![(0.0,0.2),(1.0,0.9)]; b.trans.stems=[StemMode::Swap;4];
            if arranged { a.set_sections(vec![(0.0,40.0),(50.0,130.0)]); b.set_sections(vec![(0.0,40.0),(50.0,130.0)]); }
            m.tracks=vec![a,b]; let original=m.clone();
            let right=m.split(1,200.0).unwrap(); m.split(right,220.0).unwrap(); m.split(0,190.0).unwrap();
            assert!(!m.resolve_ratios());
            assert_eq!(m.tempo_curve(),original.tempo_curve());
            assert_eq!(m.logical_transitions().len(),1);
            let restored:Mix=serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
            for &(a,b,s,e) in &restored.transitions() {
                assert_eq!((s,e),(160.0,240.0)); assert_ne!(restored.tracks[a].path,restored.tracks[b].path);
                assert_eq!(restored.tracks[b].trans.preset,Preset::BassSwap);
                assert_eq!(restored.tracks[b].trans.curve_in,original.tracks[1].trans.curve_in);
                assert_eq!(restored.tracks[b].trans.stems,[StemMode::Swap;4]);
            }
            assert!(restored.transitions().iter().any(|x| x.1==right));
            assert_eq!(original.transitions(),vec![(0,1,160.0,240.0)]);
        }
    }
    #[test]
    fn ramp_ratio_threshold_cycle_pins_only_oscillating_auto_tracks() {
        let mut m=Mix::default(); m.tempo_mode=3; m.ramp=(70.0,180.0);
        m.tracks=vec![track(60.0,0.0),track(60.0,100.0),track(70.0,200.0)];
        m.relayout(&[0,1,2],16.0);
        assert_eq!(m.tracks[0].trans.ratio,0,"unaffected Auto ratio should stay Auto");
        assert_ne!(m.tracks[1].trans.ratio,0); assert_ne!(m.tracks[2].trans.ratio,0);
        assert!(!m.resolve_ratios());
        let geometry:Vec<_>=m.tracks.iter().map(|t|(t.start_beat,t.end(m.bpm))).collect();
        m.relayout(&[0,1,2],16.0);
        assert_eq!(geometry,m.tracks.iter().map(|t|(t.start_beat,t.end(m.bpm))).collect::<Vec<_>>());
    }
    #[test]
    fn selective_stabilization_keeps_the_middle_fragment_variable() {
        let beats=crate::testgen::ramp_times(120.0,132.0,60.0,0.5);
        let mut t=track(126.0,0.0);t.duration=61.5;t.offset=0.5;t.map=Some(Arc::new(BeatMap{bpm:126.0,beats:beats.clone()}));
        let mut m=Mix::default();m.bpm=126.0;m.tempo_mode=1;m.tracks.push(t);
        let middle=m.split(0,32.5).unwrap();let outro=m.split(middle,(beats.len()-1) as f64-32.5).unwrap();
        m.tracks[0].stabilize=true;m.tracks[outro].stabilize=true;
        let residual=|from:i32,to:i32| {
            let times:Vec<_>=(from..to).map(|b|m.secs(0.0,b as f64)).collect();let n=times.len() as f64;
            let sx=(n-1.0)*n*0.5;let sy: f64=times.iter().sum();
            let sxx=(n-1.0)*n*(2.0*n-1.0)/6.0;let sxy:f64=times.iter().enumerate().map(|(i,t)|i as f64*t).sum();
            let slope=(n*sxy-sx*sy)/(n*sxx-sx*sx);let offset=(sy-slope*sx)/n;
            times.iter().enumerate().map(|(i,t)|(t-offset-slope*i as f64).abs()).fold(0.0f64,f64::max)
        };
        assert!(residual(36,91)>0.050,"middle timing was flattened by the intro's stabilization policy");
        assert!(residual(4,29)<0.002);assert!(residual(99,124)<0.002);
        assert!(m.bpm_at(85.0)-m.bpm_at(40.0)>2.0);
    }
    #[test]
    fn split_preserves_variable_auto_tempo_and_stabilized_average() {
        for stabilize in [false,true] {
            let mut m=Mix::default();m.tempo_mode=1;m.bpm=120.0;
            m.tracks=vec![track(120.0,0.0),track(120.0,160.0)];
            for t in &mut m.tracks { t.stabilize=stabilize;t.map=Some(Arc::new(BeatMap{bpm:120.0,beats:(0..260).map(|i|i as f64*0.5+0.04*(i as f64*0.07).sin()).collect()})); }
            let original=m.tempo_curve();m.split(1,200.0).unwrap();m.split(0,190.0).unwrap();
            let curve=m.tempo_curve();
            for beat in 0..400 { assert!((bpm_on(&curve,120.0,beat as f64)-bpm_on(&original,120.0,beat as f64)).abs()<1e-9,"stabilize {stabilize}, beat {beat}"); }
        }
    }
    #[test]
    fn manual_and_ramp_layout_are_stable_at_final_positions() {
        for mode in [2,3] {
            let mut m=Mix::default(); m.tempo_mode=mode; m.bpm=120.0; m.ramp=(70.0,180.0); m.tempo_pts=vec![(0.0,70.0),(900.0,180.0)];
            m.tracks=vec![track(70.0,800.0),track(140.0,0.0),track(80.0,100.0)];
            m.relayout(&[0,2,1],16.0); let before=m.transitions();
            assert!(!m.resolve_ratios(),"mode {mode} changed ratios on next frame"); assert_eq!(m.transitions(),before);
            let positions:Vec<_>=m.tracks.iter().map(|t|t.start_beat).collect();
            m.relayout(&[0,2,1],16.0); assert_eq!(positions,m.tracks.iter().map(|t|t.start_beat).collect::<Vec<_>>());
        }
    }
    #[test]
    fn phrase_ranges_follow_ratio_repeats_deletions_and_trim() {
        let mut m=Mix::default(); let mut t=track(120.0,10.0); t.beat_mult=2.0; t.trim_in=2.0; t.trim_out=100.0;
        t.set_sections(vec![(2.0,8.0),(2.0,8.0),(12.0,16.0)]);
        m.tracks.push(t);
        // Source phrase beat 8 is source second 4; each repeated occurrence must be mapped.
        let ranges=m.tracks[0].phrase_ranges(&[(0.0,0),(8.0,1),(20.0,4)]);
        let boundaries:Vec<_>=ranges.iter().filter(|r|r.0==1).map(|r|r.2).collect();
        assert_eq!(boundaries,vec![26.0,50.0]);
        assert!(!ranges.iter().any(|r|r.2<18.0 || r.3>82.0));
    }
    #[test]
    fn phrase_layout_aligns_last_visible_outro_to_first_visible_intro() {
        let mut m=Mix::default(); m.bpm=120.0;
        let mut a=track(120.0,0.0); a.trans.ratio=2; a.set_sections(vec![(0.0,60.0),(40.0,60.0)]);
        let mut b=track(120.0,0.0); b.trans.ratio=3;
        m.tracks=vec![a,b]; let phrases=vec![vec![(0.0,0),(112.0,4)],vec![(8.0,0),(64.0,4)]];
        assert_eq!(m.relayout_phrases(&[0,1],16.0,&phrases),1);
        assert_eq!(m.tracks[1].start_beat,300.0);
        assert!(!m.resolve_ratios());
        m.tracks[0].set_sections(vec![(0.0,50.0),(0.0,50.0)]);
        assert_eq!(m.relayout_phrases(&[0,1],16.0,&phrases),0,"deleted outro cannot anchor placement");
    }
    #[test]
    fn editing_one_fragment_synchronizes_its_transition_siblings() {
        let mut m=Mix::default(); m.tracks=vec![track(120.0,0.0),track(120.0,160.0)];
        let right=m.split(1,200.0).unwrap(); m.tracks[right].trans.preset=Preset::EchoOut;
        m.tracks[right].trans.curve_out=vec![(0.0,0.8),(1.0,0.1)]; m.sync_transition(right);
        assert_eq!(m.tracks[1].trans.preset,Preset::EchoOut);
        assert_eq!(m.tracks[1].trans.curve_out,m.tracks[right].trans.curve_out);
        assert_eq!(m.tracks[0].trans.preset,Preset::Crossfade);
    }
    #[test]
    fn linear_tempo_seconds_are_exact_and_reversible() {
        let c=vec![(0.0,120.0),(16.0,240.0)]; let expected=8.0*2.0_f64.ln();
        assert!((secs_on(&c,120.0,0.0,16.0)-expected).abs()<1e-12);
        assert!((secs_on(&c,120.0,16.0,0.0)+expected).abs()<1e-12);
    }
    #[test]
    fn automation_replaces_a_point_at_the_same_time() {
        let mut t=track(120.0,0.0);
        for v in [0.1,0.5,0.9] { t.auto_set(0,8.0,v); }
        assert_eq!(t.auto[0],vec![(8.0,0.9)]);
    }
    #[test]
    fn split_keeps_explicit_timing_ratio() {
        for arranged in [false,true] {
            let mut m=Mix::default(); m.bpm=120.0;
            let mut t=track(80.0,0.0); t.trans.ratio=4;
            if arranged { t.set_sections(vec![(0.0,40.0),(50.0,120.0)]); }
            m.tracks.push(t); m.resolve_ratios();
            let end=m.tracks[0].end(m.bpm);
            let right=m.split(0,100.0).unwrap(); m.resolve_ratios();
            assert!((m.tracks[right].vis_start(m.bpm)-100.0).abs()<1e-8);
            assert!((m.tracks[right].end(m.bpm)-end).abs()<1e-8);
        }
    }
}
pub fn bpm_on(c: &[(f64, f64)], bpm: f64, b: f64) -> f64 {
    if c.is_empty() { bpm } else { raw_bpm_on(c,bpm,b).max(20.0) }
}
fn raw_bpm_on(c: &[(f64,f64)],bpm:f64,b:f64) -> f64 {
    if c.is_empty() { return bpm; }
    let (f, l) = (c[0], c[c.len() - 1]); if b <= f.0 { return f.1; } if b >= l.0 { return l.1; }
    let i = c.partition_point(|q| q.0 <= b); let (p, q) = (c[i - 1], c[i]); p.1 + (q.1 - p.1) * (b - p.0) / (q.0 - p.0).max(1e-9)
}
pub fn secs_on(c: &[(f64, f64)], bpm: f64, a: f64, b: f64) -> f64 {
    if b<a { return -secs_on(c,bpm,b,a); }
    if c.is_empty() { return (b-a)*60.0/bpm; }
    let mut cursor=a; let mut seconds=0.0;
    for end in c.iter().map(|p|p.0).filter(|x|*x>a && *x<b).chain(std::iter::once(b)) {
        let (lo,hi)=(raw_bpm_on(c,bpm,cursor),raw_bpm_on(c,bpm,end));
        let integral=|a:f64,b:f64,len:f64| if (b-a).abs()<1e-10 { 60.0*len/a } else { 60.0*len*((b-a)/a).ln_1p()/(b-a) };
        let len=end-cursor;
        seconds+=if lo<=20.0 && hi<=20.0 { 3.0*len }
            else if lo>=20.0 && hi>=20.0 { integral(lo,hi,len) }
            else {
                let fraction=(20.0-lo)/(hi-lo);
                if lo<20.0 { 3.0*len*fraction+integral(20.0,hi,len*(1.0-fraction)) }
                else { integral(lo,20.0,len*fraction)+3.0*len*(1.0-fraction) }
            };
        cursor=end;
    }
    seconds
}

#[cfg(test)]
mod precise_grid_controls {
    use super::*;
    fn track()->Track { serde_json::from_str(r#"{"path":"test.wav","title":"Test","artist":"Test","bpm":120.0,"offset":0.25,"key":"8A","duration":24.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap() }
    #[test]
    fn grid_scope_keeps_placement_and_unselected_beats_anchored() {
        for scope in [GridScope::All,GridScope::Single,GridScope::Following] {
            let mut t=track();let before=t.clone();let d=t.adjust_grid(scope,16.0,0.010).unwrap();assert!((d-0.010).abs()<1e-8);
            assert_eq!(t.start_beat,before.start_beat);
            assert!((t.sec_at(16.0)-before.sec_at(16.0)-0.010).abs()<1e-8);
            if scope!=GridScope::All { assert_eq!(t.sec_at(12.0),before.sec_at(12.0)); }
            if scope==GridScope::Single { assert_eq!(t.sec_at(15.0),before.sec_at(15.0));assert_eq!(t.sec_at(17.0),before.sec_at(17.0)); }
            if scope==GridScope::Following { assert!(t.sec_at(24.0)>before.sec_at(24.0)+0.010); }
        }
    }
    #[test]
    fn single_beat_preserves_fractional_analysis_anchor_and_clamps_safely() {
        let mut t=track(); t.offset=0.361;
        t.map=Some(Arc::new(BeatMap {beats:(0..64).map(|i|0.25+i as f64*0.5+(i as f64*0.3).sin()*0.005).collect(),bpm:120.0}));
        let old=t.clone(); t.adjust_grid(GridScope::Single,16.0,0.010).unwrap();
        for k in 0..40 { if k!=16 { assert!((t.sec_at(k as f64)-old.sec_at(k as f64)).abs()<1e-9,"beat {k} moved"); } }
        for _ in 0..8 { t.adjust_grid(GridScope::Single,16.0,0.25).unwrap(); }
        assert!(t.sec_at(16.0)<t.sec_at(17.0)); assert!(t.sec_at(16.0)>t.sec_at(15.0));
        let mut t=track(); t.beat_mult=2.0; let old=serde_json::to_string(&t).unwrap();
        assert!(t.adjust_grid(GridScope::Single,16.0,0.01).is_err()); assert_eq!(serde_json::to_string(&t).unwrap(),old);
    }
    #[test]
    fn arranged_sections_and_invalid_targets_are_protected_without_partial_edits() {
        let mut t=track();t.sections=vec![(0.0,4.0),(0.0,4.0)];let before=serde_json::to_string(&t).unwrap();
        assert!(t.adjust_grid(GridScope::All,4.0,0.01).is_err());assert_eq!(serde_json::to_string(&t).unwrap(),before);
        let mut t=track();let before=serde_json::to_string(&t).unwrap();assert!(t.adjust_grid(GridScope::Single,1000.0,0.01).is_err());assert_eq!(serde_json::to_string(&t).unwrap(),before);
    }
}

#[cfg(test)]
mod protected_transition_tests {
    use super::*;
    #[test]
    fn protected_transition_survives_layout_shift_and_ratio_resolution() {
        let mut m:Mix=serde_json::from_str(include_str!("../tests/fixtures/unstable-flex-pair.json")).unwrap();
        assert!(!m.has_transition_locks(),"legacy project unexpectedly froze arrangement");
        for t in &mut m.tracks {t.set_grid_mode(1);}
        let mut value=serde_json::to_value(&m).unwrap();value["tracks"][1]["trans"]["locked"]=serde_json::json!(true);
        let mut m:Mix=serde_json::from_value(value).unwrap();let before=serde_json::to_string(&m).unwrap();
        m.set_edge(0,false,32.0);
        assert!(m.split(0,16.0).is_none());
        m.move_to_lane(0,2);
        m.shift_logical_transition(1,16.0,true);
        m.relayout(&[1,0],4.0);
        m.resolve_ratios();
        assert_eq!(serde_json::to_string(&m).unwrap(),before,"protected arrangement was changed");
    }
}
