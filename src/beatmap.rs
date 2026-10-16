//! Dynamic beat grids: a per-track beat map (beat index -> source seconds, monotonic) and the
//! mapping between mix beats and source time that the engine, timeline and layout share.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn moving_an_overcrowded_beat_keeps_the_map_unchanged() {
        let mut m=BeatMap {bpm:120.0,beats:(0..16).map(|i|i as f64*0.0001).collect()};
        let before=m.beats.clone();
        assert_eq!(m.move_single(1,0.010),0.0);
        assert_eq!(m.beats,before);
    }
}

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
pub struct Grid { map: Option<Arc<BeatMap>>, r: f64, u0: f64, bpm: f64, offset: f64, start: f64, regions: Arc<Vec<(f64, f64, f64)>>, revision: u64, outer: Option<(f64,f64,[bool;2])> }

impl Grid {
    pub fn new(map: Option<Arc<BeatMap>>, bpm: f64, offset: f64, start: f64) -> Grid {
        // the map is used while the track BPM is the analysed one (or exactly x2 / /2 of it);
        // a hand-edited BPM (drag, tap tempo) means the user takes over with a fixed grid
        let (map, r) = match map.filter(|m| m.beats.len() >= 8 && m.bpm > 0.0) {
            Some(m) => { let q = m.bpm / bpm; match [1.0, 0.5, 2.0].into_iter().find(|c| (q / c - 1.0).abs() < 1e-9) { Some(r) => (Some(m), r), None => (None, 1.0) } }
            None => (None, 1.0),
        };
        let u0 = map.as_ref().map(|m| m.u_of(offset)).unwrap_or(0.0);
        let revision = map.as_ref().map_or(0, |m| Arc::as_ptr(m) as u64);
        Grid { map, r, u0, bpm, offset, start, regions: Arc::new(vec![]), revision, outer: None }
    }
    /// Beat ratio: one mix beat covers `1/m` track beats (m = 2: a 70 BPM track runs on a 140 BPM grid).
    pub fn scaled(mut self, m: f64) -> Grid { if m > 0.0 && m != 1.0 { self.r /= m; self.bpm *= m; } self }
    pub fn dynamic(&self) -> bool { self.map.is_some() }
    pub fn ratio(&self) -> f64 { self.r }
    /// Mix beat -> source seconds.
    #[inline] fn raw_sec(&self, b: f64) -> f64 { let x = b - self.start; match &self.map { Some(m) => m.t_of(self.u0 + x * self.r), None => self.offset + x * 60.0 / self.bpm } }
    /// Ordered (destination beat, source beat, length) spans. Source beat maps remain monotonic.
    pub fn arranged(mut self, sections: &[(f64, f64)], trim: f64) -> Self {
        let mut dest = self.raw_beat(trim); let mut regions = vec![];
        let mut hash = 0xcbf29ce484222325u64 ^ self.revision;
        for &(a,b) in sections {
            let source = self.raw_beat(a); let len = self.raw_beat(b) - source;
            if len.is_finite() && len > 1e-9 {
                regions.push((dest, source, len)); dest += len;
                hash = (hash ^ a.to_bits()).wrapping_mul(0x100000001b3);
                hash = (hash ^ b.to_bits()).wrapping_mul(0x100000001b3);
            }
        }
        if !regions.is_empty() { self.revision = hash; self.regions = Arc::new(regions); } self
    }
    fn region(&self, b: f64) -> usize { self.regions.partition_point(|r| r.0 <= b).saturating_sub(1) }
    pub fn section_id(&self, b: f64) -> u64 { self.revision.wrapping_add(self.region(b) as u64) }
    pub fn outer_fades(mut self,start:f64,end:f64,flags:[bool;2]) -> Self {
        if flags.iter().any(|v|*v) { self.outer=Some((start,end,flags)); }self
    }
    pub fn splice_gain(&self, b: f64) -> f64 {
        let width=self.bpm/60.0*0.003;
        let mut gain=1.0_f64;
        if !self.regions.is_empty() {
            let i=self.region(b);let r=self.regions[i];
            let left=if i>0 { (b-r.0)/width } else { 1.0 };
            let right=if i+1<self.regions.len() { (r.0+r.2-b)/width } else { 1.0 };
            gain=left.min(right);
        }
        if let Some((start,end,flags))=self.outer {
            if flags[0] { gain=gain.min((b-start)/width); }
            if flags[1] { gain=gain.min((end-b)/width); }
        }
        gain.clamp(0.0,1.0)
    }
    pub fn cuts(&self) -> impl Iterator<Item=f64> + '_ { self.regions.iter().skip(1).map(|r| r.0) }
    /// Evaluate within the section containing `at`, also for WSOLA pre-roll outside its edges.
    #[inline] pub fn region_sec(&self, b: f64, at: f64) -> f64 {
        let source = self.regions.get(self.region(at)).map(|r| r.1 + b - r.0).unwrap_or(b);
        self.raw_sec(source)
    }
    #[inline] pub fn sec(&self, b: f64) -> f64 { self.region_sec(b, b) }
    pub fn shares_source_map(&self,other:&Self)->bool {
        self.r==other.r && self.bpm==other.bpm && match (&self.map,&other.map) {
            (Some(a),Some(b))=>Arc::ptr_eq(a,b),(None,None)=>true,_=>false
        }
    }
    /// Inverse inside a chosen arranged occurrence, including its lookahead.
    #[inline] pub fn region_beat(&self, sec: f64, at: f64) -> f64 {
        let source=self.raw_beat(sec);
        self.regions.get(self.region(at)).map_or(source,|r|r.0+source-r.1)
    }
    /// Every arranged occurrence of a source marker; no occurrence when the source was deleted.
    pub fn occurrences(&self, s: f64) -> Vec<f64> {
        let source = self.raw_beat(s);
        if self.regions.is_empty() { return vec![source]; }
        self.regions.iter().filter(|r| source >= r.1 && source < r.1+r.2).map(|r| r.0+source-r.1).collect()
    }
    pub fn beat(&self, s: f64) -> f64 { self.occurrences(s).first().copied().unwrap_or_else(|| self.raw_beat(s)) }
    /// Source seconds -> mix beat.
    fn raw_beat(&self, s: f64) -> f64 { self.start + match &self.map { Some(m) => (m.u_of(s) - self.u0) / self.r, None => (s - self.offset) * self.bpm / 60.0 } }
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
        if lo>hi || !d.is_finite() { return 0.0; }
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
