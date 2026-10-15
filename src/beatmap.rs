//! Dynamic beat grids: a per-track beat map (beat index -> source seconds, monotonic) and the
//! mapping between mix beats and source time that the engine, timeline and layout share.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct BeatMap {
    /// Time (s) of every tracked beat, strictly increasing.
    pub beats: Vec<f64>,
    /// Global BPM this map was analysed with (the track BPM it belongs to).
    pub bpm: f64,
}

impl BeatMap {
    fn edge_period(&self, end: bool) -> f64 {
        let n = self.beats.len(); let k = 8.min(n - 1);
        if end { (self.beats[n - 1] - self.beats[n - 1 - k]) / k as f64 } else { (self.beats[k] - self.beats[0]) / k as f64 }
    }
    /// Fractional beat index -> seconds (linear between beats, linear extrapolation outside).
    pub fn t_of(&self, u: f64) -> f64 {
        let n = self.beats.len();
        if u <= 0.0 { return self.beats[0] + u * self.edge_period(false); }
        if u >= (n - 1) as f64 { return self.beats[n - 1] + (u - (n - 1) as f64) * self.edge_period(true); }
        let i = u as usize; let f = u - i as f64;
        self.beats[i] + f * (self.beats[i + 1] - self.beats[i])
    }
    /// Seconds -> fractional beat index (inverse of `t_of`).
    pub fn u_of(&self, s: f64) -> f64 {
        let n = self.beats.len();
        if s <= self.beats[0] { return (s - self.beats[0]) / self.edge_period(false); }
        if s >= self.beats[n - 1] { return (n - 1) as f64 + (s - self.beats[n - 1]) / self.edge_period(true); }
        let i = self.beats.partition_point(|&b| b <= s) - 1;
        i as f64 + (s - self.beats[i]) / (self.beats[i + 1] - self.beats[i])
    }
    /// (min, max) local tempo over 16-beat windows.
    pub fn tempo_range(&self) -> (f64, f64) {
        let mut r = (f64::MAX, f64::MIN);
        for w in self.beats.windows(17) { let b = 16.0 * 60.0 / (w[16] - w[0]); r = (r.0.min(b), r.1.max(b)); }
        if r.0 > r.1 { (self.bpm, self.bpm) } else { r }
    }
}

/// Mix beat <-> source seconds for one track. Track beat x = mix beat - start_beat; x = 0 is the
/// downbeat at `offset` seconds. With a beat map the spacing follows the map; otherwise linear.
#[derive(Clone)]
pub struct Grid { map: Option<Arc<BeatMap>>, r: f64, u0: f64, bpm: f64, offset: f64, start: f64 }

impl Grid {
    pub fn new(map: Option<Arc<BeatMap>>, bpm: f64, offset: f64, start: f64) -> Grid {
        // the map is used while the track BPM is the analysed one (or exactly x2 / /2 of it);
        // a hand-edited BPM (drag, tap tempo) means the user takes over with a fixed grid
        let (map, r) = match map.filter(|m| m.beats.len() >= 8 && m.bpm > 0.0) {
            Some(m) => { let q = m.bpm / bpm; match [1.0, 0.5, 2.0].into_iter().find(|c| (q / c - 1.0).abs() < 1e-9) { Some(r) => (Some(m), r), None => (None, 1.0) } }
            None => (None, 1.0),
        };
        let u0 = map.as_ref().map(|m| m.u_of(offset)).unwrap_or(0.0);
        Grid { map, r, u0, bpm, offset, start }
    }
    /// Beat ratio: one mix beat covers `1/m` track beats (m = 2: a 70 BPM track runs on a 140 BPM grid).
    pub fn scaled(mut self, m: f64) -> Grid { if m > 0.0 && m != 1.0 { self.r /= m; self.bpm *= m; } self }
    pub fn dynamic(&self) -> bool { self.map.is_some() }
    pub fn ratio(&self) -> f64 { self.r }
    /// Mix beat -> source seconds.
    #[inline] pub fn sec(&self, b: f64) -> f64 { let x = b - self.start; match &self.map { Some(m) => m.t_of(self.u0 + x * self.r), None => self.offset + x * 60.0 / self.bpm } }
    /// Source seconds -> mix beat.
    pub fn beat(&self, s: f64) -> f64 { self.start + match &self.map { Some(m) => (m.u_of(s) - self.u0) / self.r, None => (s - self.offset) * self.bpm / 60.0 } }
}

/// Beatgrid editor operations. Every edit keeps the beats strictly increasing.
impl BeatMap {
    /// Build an explicit beat map (one entry per track beat) from any grid, so a fixed grid or
    /// a x2 / /2 map can be edited beat by beat. `start` is the track's start beat.
    pub fn from_grid(g: &Grid, start: f64, dur: f64, bpm: f64) -> BeatMap {
        let lo = (g.beat(0.0) - start).floor() as i64 - 1; let hi = (g.beat(dur) - start).ceil() as i64 + 1;
        BeatMap { beats: (lo..=hi).map(|k| g.sec(start + k as f64)).collect(), bpm }
    }
    /// Shift every beat by `d` seconds.
    pub fn shift(&mut self, d: f64) { for b in &mut self.beats { *b += d; } }
    /// Move beat `i` alone by `d` seconds, clamped 5 ms inside its neighbours. Returns the applied shift.
    pub fn move_single(&mut self, i: usize, d: f64) -> f64 {
        let n = self.beats.len(); if i >= n { return 0.0; }
        let lo = if i > 0 { self.beats[i - 1] + 0.005 } else { f64::MIN };
        let hi = if i + 1 < n { self.beats[i + 1] - 0.005 } else { f64::MAX };
        let old = self.beats[i]; self.beats[i] = (old + d).clamp(lo, hi); self.beats[i] - old
    }
    /// Move beat `i` by `d` seconds and stretch everything after the pivot (`pivot` < i stays put,
    /// as do all beats before it). Returns the stretch factor applied to the beat spacing.
    pub fn stretch_following(&mut self, pivot: usize, i: usize, d: f64) -> f64 {
        let n = self.beats.len(); if i >= n { return 1.0; }
        if pivot >= i { for b in &mut self.beats[i..] { *b += d; } return 1.0; }
        let tp = self.beats[pivot]; let span = self.beats[i] - tp;
        let s = ((span + d) / span).clamp(0.25, 4.0);
        for b in &mut self.beats[pivot + 1..] { *b = tp + (*b - tp) * s; }
        s
    }
    /// Drop the beats before `i` (the grid extrapolates backwards from the new first beat).
    pub fn trim_before(&mut self, i: usize) { if i > 0 && self.beats.len() - i >= 8 { self.beats.drain(..i); } }
}
