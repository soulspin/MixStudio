//! Effect blocks on the timeline (reverb, echo/delay, flanger, filter sweep) and their DSP.
//! A block feeds its effect between `start` and `start + len` (mix beats); reverb and echo keep
//! ringing for `tail` beats after that, then fade out. All state is preallocated.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug, Default)]
pub enum FxKind { #[default] Reverb, Echo, Flanger, FilterSweep }
impl FxKind {
    pub const ALL: [FxKind; 4] = [FxKind::Reverb, FxKind::Echo, FxKind::Flanger, FxKind::FilterSweep];
    pub fn name(self) -> &'static str { match self { FxKind::Reverb => "Reverb", FxKind::Echo => "Echo / delay", FxKind::Flanger => "Flanger", FxKind::FilterSweep => "Filter sweep" } }
    pub fn has_tail(self) -> bool { matches!(self, FxKind::Reverb | FxKind::Echo) }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FxBlock {
    pub kind: FxKind,
    /// 0/1 = Deck 1/2, 2/3 = Sample 1/2, 4 = master
    pub target: usize,
    pub start: f64, pub len: f64, pub tail: f64,
    /// wet amount 0..1
    pub mix: f32,
    /// reverb size / echo feedback, 0..1
    pub decay: f32,
    /// flanger depth / filter sweep depth, 0..1
    pub depth: f32,
    /// echo time in beats / flanger LFO period in beats
    pub time: f32,
}
impl FxBlock {
    pub fn new(kind: FxKind, target: usize, start: f64) -> FxBlock {
        let (mix, decay, depth, time, tail) = match kind { FxKind::Reverb => (0.4, 0.6, 0.5, 1.0, 8.0), FxKind::Echo => (0.45, 0.5, 0.5, 0.75, 8.0), FxKind::Flanger => (0.6, 0.5, 0.7, 4.0, 0.0), FxKind::FilterSweep => (1.0, 0.5, 0.8, 1.0, 0.0) };
        FxBlock { kind, target, start, len: 8.0, tail, mix, decay, depth, time }
    }
    pub fn end(&self) -> f64 { self.start + self.len + if self.kind.has_tail() { self.tail } else { 0.0 } }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FxPreset { pub name: String, pub block: FxBlock }
pub fn presets_path() -> PathBuf { crate::library::config_dir().join("fx-presets.json") }
pub fn load_presets() -> Vec<FxPreset> { std::fs::read_to_string(presets_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }
pub fn save_presets(v: &[FxPreset]) -> std::io::Result<()> {
    let p = presets_path(); if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
    let tmp = p.with_extension("json.tmp"); std::fs::write(&tmp, serde_json::to_string_pretty(v).unwrap())?; std::fs::rename(tmp, p)
}

const COMB: [usize; 4] = [1116, 1188, 1277, 1356];
const AP: [usize; 2] = [556, 441];

/// Effect state for one block (preallocated for the engine sample rate).
pub struct FxState {
    comb: [[Vec<f32>; 4]; 2], cpos: [[usize; 4]; 2], cfilt: [[f32; 4]; 2], ap: [[Vec<f32>; 2]; 2], apos: [[usize; 2]; 2],
    dl: Vec<[f32; 2]>, dpos: usize, fl: Vec<[f32; 2]>, fpos: usize, svf: [[f32; 2]; 2], pub active: bool, sr: f32,
}
impl FxState {
    pub fn new(sr: f32) -> FxState {
        let k = sr / 44100.0;
        let comb = [0, 1].map(|c| COMB.map(|n| vec![0.0; ((n + 23 * c) as f32 * k) as usize + 1]));
        let ap = [0, 1].map(|c| AP.map(|n| vec![0.0; ((n + 23 * c) as f32 * k) as usize + 1]));
        FxState { comb, cpos: [[0; 4]; 2], cfilt: [[0.0; 4]; 2], ap, apos: [[0; 2]; 2], dl: vec![[0.0; 2]; (sr * 4.0) as usize], dpos: 0, fl: vec![[0.0; 2]; 2048], fpos: 0, svf: [[0.0; 2]; 2], active: false, sr }
    }
    pub fn reset(&mut self) {
        for c in 0..2 { for v in &mut self.comb[c] { v.iter_mut().for_each(|x| *x = 0.0); } for v in &mut self.ap[c] { v.iter_mut().for_each(|x| *x = 0.0); } }
        self.cfilt = [[0.0; 4]; 2]; self.dl.iter_mut().for_each(|x| *x = [0.0; 2]); self.fl.iter_mut().for_each(|x| *x = [0.0; 2]); self.svf = [[0.0; 2]; 2]; self.active = false;
    }
    /// Process `buf` in place for mix beats b0 + i * bpb. `live` ignores the block's time range.
    pub fn process(&mut self, b: &FxBlock, buf: &mut [[f32; 2]], b0: f64, bpb: f64, bpm: f64, live: bool) {
        let n = buf.len(); let b1 = b0 + bpb * n as f64;
        if !live && (b1 < b.start || b0 > b.end() + 0.5) { if self.active { self.reset(); } return; }
        self.active = true;
        let sr = self.sr; let send_end = b.start + b.len; let ramp = 0.02;
        for i in 0..n {
            let t = b0 + bpb * i as f64;
            let send = if live { 1.0 } else { (((t - b.start) / ramp).min((send_end - t) / ramp)).clamp(0.0, 1.0) as f32 };
            let gate = if live { 1.0 } else { (((t - b.start + ramp) / ramp).min((b.end() + 0.5 - t) / 0.5)).clamp(0.0, 1.0) as f32 };
            let x = buf[i];
            match b.kind {
                FxKind::Reverb => {
                    let fb = 0.70 + 0.28 * b.decay.clamp(0.0, 1.0); let damp = 0.3;
                    for c in 0..2 {
                        let inp = x[c] * send * 0.25; let mut acc = 0.0;
                        for k in 0..4 { let v = &mut self.comb[c][k]; let p = self.cpos[c][k]; let y = v[p];
                            self.cfilt[c][k] = y * (1.0 - damp) + self.cfilt[c][k] * damp; v[p] = inp + self.cfilt[c][k] * fb; self.cpos[c][k] = (p + 1) % v.len(); acc += y; }
                        for k in 0..2 { let v = &mut self.ap[c][k]; let p = self.apos[c][k]; let bo = v[p]; let y = -acc + bo; v[p] = acc + bo * 0.5; self.apos[c][k] = (p + 1) % v.len(); acc = y; }
                        buf[i][c] = x[c] + acc * b.mix * gate;
                    }
                }
                FxKind::Echo => {
                    let d = ((b.time.max(0.05) as f64 * 60.0 / bpm) as f32 * sr) as usize; let d = d.clamp(1, self.dl.len() - 1);
                    let rp = (self.dpos + self.dl.len() - d) % self.dl.len(); let e = self.dl[rp]; let fb = 0.9 * b.decay.clamp(0.0, 1.0);
                    self.dl[self.dpos] = [x[0] * send + e[1] * fb, x[1] * send + e[0] * fb]; self.dpos = (self.dpos + 1) % self.dl.len();
                    for c in 0..2 { buf[i][c] = x[c] + e[c] * b.mix * gate; }
                }
                FxKind::Flanger => {
                    let ph = (t / b.time.max(0.25) as f64).fract() as f32 * std::f32::consts::TAU;
                    let dsec = 0.001 + 0.004 * b.depth.clamp(0.0, 1.0) * (0.5 + 0.5 * ph.sin()); let d = dsec * sr; let len = self.fl.len();
                    let di = d.floor() as usize; let fr = d - di as f32;
                    for c in 0..2 {
                        let r0 = self.fl[(self.fpos + len - di) % len][c]; let r1 = self.fl[(self.fpos + len - di - 1) % len][c]; let y = r0 * (1.0 - fr) + r1 * fr;
                        self.fl[self.fpos][c] = x[c] + y * 0.5;
                        let m = b.mix * send * 0.5; buf[i][c] = x[c] * (1.0 - m) + y * m;
                    }
                    self.fpos = (self.fpos + 1) % len;
                }
                FxKind::FilterSweep => {
                    let prog = if live { b.time.clamp(0.0, 1.0) as f64 } else { ((t - b.start) / b.len.max(0.01)).clamp(0.0, 1.0) };
                    let lo = 150.0 + (1.0 - b.depth.clamp(0.0, 1.0)) * 4000.0;
                    let fc = (20000f32.ln() + (lo.ln() - 20000f32.ln()) * prog as f32).exp().min(sr * 0.2);
                    let f = 2.0 * (std::f32::consts::PI * fc / sr).sin(); let q = 0.7;
                    for c in 0..2 { let s = &mut self.svf[c]; let hp = x[c] - s[1] - q * s[0]; s[0] += f * hp; s[1] += f * s[0];
                        let wet = s[1]; let m = b.mix * send; buf[i][c] = x[c] * (1.0 - m) + wet * m; }
                }
            }
        }
    }
}
