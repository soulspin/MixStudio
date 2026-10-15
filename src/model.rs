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
pub struct Transition { pub preset: Preset, pub stems: [StemMode; 4] }

#[derive(Clone, Serialize, Deserialize)]
pub struct Track {
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
}
impl Track {
    pub fn grid(&self) -> Grid { Grid::new(self.map.clone(), self.bpm, self.offset, self.start_beat) }
    /// mix beat -> source seconds
    pub fn sec_at(&self, b: f64) -> f64 { self.grid().sec(b) }
    pub fn vis_start(&self, _mix_bpm: f64) -> f64 { self.grid().beat(self.trim_in) }
    pub fn end(&self, _mix_bpm: f64) -> f64 { self.grid().beat(self.duration - self.trim_out) }
    /// beats from the file start to the first downbeat
    pub fn intro_beats(&self) -> f64 { self.start_beat - self.grid().beat(0.0) }
    /// Make sure the track has an editable beat map that maps one track beat to one map entry.
    pub fn ensure_map(&mut self) {
        let g = self.grid(); if g.dynamic() && g.ratio() == 1.0 { return; }
        let m = BeatMap::from_grid(&g, self.start_beat, self.duration, self.bpm);
        self.map = Some(Arc::new(m));
    }
    /// Map index of the track beat at mix beat `start_beat + k` (call `ensure_map` first).
    pub fn map_index(&self, k: i64) -> usize {
        let s = self.sec_at(self.start_beat + k as f64);
        self.map.as_ref().map(|m| m.u_of(s).round().max(0.0) as usize).unwrap_or(0).min(self.map.as_ref().map(|m| m.beats.len() - 1).unwrap_or(0))
    }
    /// Edit the beat map; the downbeat stays on the same beat, so the engine and the timeline follow.
    pub fn edit_map<R>(&mut self, f: impl FnOnce(&mut BeatMap) -> R) -> R {
        self.ensure_map();
        let mut m = (**self.map.as_ref().unwrap()).clone();
        let k = (m.u_of(self.offset).round().max(0.0) as usize).min(m.beats.len() - 1);
        let r = f(&mut m);
        self.offset = m.beats[k]; self.map = Some(Arc::new(m)); self.normalize_grid(); r
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
    #[serde(default = "yes")] pub key_lock: bool, #[serde(default)] pub metronome: bool, #[serde(default = "half")] pub metro_vol: f32, #[serde(default)] pub auto_stems: bool }
fn yes() -> bool { true }
fn half() -> f32 { 0.5 }
fn one() -> f32 { 1.0 }
impl Default for Mix { fn default() -> Self { Mix { name: "My mix".into(), bpm: 128.0, tracks: vec![], decks: [Deck::default(); 4], master: 1.0, key_lock: true, metronome: false, metro_vol: 0.5, auto_stems: false } } }

impl Mix {
    /// indices of main-deck tracks sorted by start
    pub fn main_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.tracks.len()).filter(|&i| self.tracks[i].lane < 2).collect();
        v.sort_by(|&a, &b| (self.tracks[a].start_beat, self.tracks[a].trim_in).partial_cmp(&(self.tracks[b].start_beat, self.tracks[b].trim_in)).unwrap());
        v
    }
    /// (out idx, in idx, start beat, end beat)
    pub fn transitions(&self) -> Vec<(usize, usize, f64, f64)> {
        let o = self.main_order();
        let mut r = vec![];
        for w in o.windows(2) {
            let (a, b) = (&self.tracks[w[0]], &self.tracks[w[1]]);
            let s = b.vis_start(self.bpm).max(a.vis_start(self.bpm));
            let e = a.end(self.bpm).min(b.end(self.bpm));
            if e > s + 0.5 { r.push((w[0], w[1], s, e)); }
        }
        r
    }
    pub fn length_beats(&self) -> f64 { self.tracks.iter().map(|t| t.end(self.bpm)).fold(0.0, f64::max) }
    /// lay main tracks out in current order with `overlap_bars` overlap, alternating decks
    pub fn relayout(&mut self, order: &[usize], overlap_bars: f64) {
        let mut cursor = 0.0f64;
        for (k, &i) in order.iter().enumerate() {
            let bpm = self.bpm;
            let t = &mut self.tracks[i];
            t.lane = k % 2;
            if k == 0 { t.start_beat = (t.intro_beats() / 4.0 - 0.01).ceil() * 4.0; }
            else { let intro = t.intro_beats(); t.start_beat = ((cursor - overlap_bars * 4.0 + intro) / 4.0).round() * 4.0; }
            cursor = t.end(bpm);
        }
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
    let p = |s: &str| -> Option<(i32, char)> { let (n, l) = s.split_at(s.len().checked_sub(1)?); Some((n.parse().ok()?, l.chars().next()?)) };
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
        let t = self.tracks.get(ti)?; let s = t.sec_at(b);
        if s <= t.trim_in + 0.05 || s >= t.duration - t.trim_out - 0.05 { return None; }
        let mut right = t.clone();
        right.trim_in = s; right.fade_in = 0.0; right.trans = Default::default(); right.locked = false;
        let left = &mut self.tracks[ti]; left.trim_out = left.duration - s; left.fade_out = 0.0;
        self.tracks.push(right); Some(self.tracks.len() - 1)
    }
    /// Move the left or right edge of a clip to mix beat `b` (trim or extend, at least one beat long).
    pub fn set_edge(&mut self, ti: usize, left: bool, b: f64) {
        let t = &mut self.tracks[ti]; let s = t.sec_at(b); let min = 60.0 / t.bpm;
        if left { t.trim_in = s.clamp(0.0, (t.duration - t.trim_out - min).max(0.0)); t.fade_in = t.fade_in.min(t.end(0.0) - t.vis_start(0.0)); }
        else { t.trim_out = (t.duration - s).clamp(0.0, (t.duration - t.trim_in - min).max(0.0)); t.fade_out = t.fade_out.min(t.end(0.0) - t.vis_start(0.0)); }
    }
    /// Copy a clip (Alt+drag on a sample); returns the copy's index.
    pub fn duplicate(&mut self, ti: usize) -> usize { let mut c = self.tracks[ti].clone(); c.locked = false; self.tracks.push(c); self.tracks.len() - 1 }
    /// Put a clip on `lane` (0/1 = decks, 2/3 = sample lanes); deck clips alternate Deck 1 / Deck 2 in time order.
    pub fn move_to_lane(&mut self, ti: usize, lane: usize) {
        self.tracks[ti].lane = lane.min(3);
        if lane < 2 { self.tracks[ti].trans = Default::default(); }
        let o = self.main_order(); for (k, &i) in o.iter().enumerate() { self.tracks[i].lane = k % 2; }
    }
}
