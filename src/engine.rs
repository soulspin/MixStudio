//! Mix renderer shared by the realtime audio thread and the offline exporter.
//! Tempo matching: with Key lock (default) each track is time-stretched by a WSOLA
//! (waveform-similarity overlap-add) stretcher implemented here, which keeps pitch.
//! Grains are anchored to the timeline (ideal input position = the track's beat map evaluated at the mix beat,
//! so the stretch ratio changes continuously and every source beat lands on a mix beat),
//! so there is no drift. Without Key lock: varispeed (cubic Hermite resampling, pitch follows tempo).
use crate::analysis::Audio;
use crate::model::{Deck, Mix, Preset, StemMode};
use crate::stems::Stems;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

pub struct SnapTrack { pub stems: Option<Arc<Stems>>, pub audio: Arc<Audio>, pub lane: usize, pub start_beat: f64, pub offset: f64, pub bpm: f64, pub vis: f64, pub end: f64, pub grid: crate::beatmap::Grid }
pub struct Snap { pub key_lock: bool, pub metro: Option<f32>, pub bpm: f64, pub tracks: Vec<SnapTrack>, pub trans: Vec<(usize, usize, f64, f64, Preset, [StemMode; 4])>, pub decks: [Deck; 4], pub master: f32 }

impl Snap {
    pub fn build(mix: &Mix, audio: &[Option<Arc<Audio>>]) -> Snap { Self::build_with(mix, audio, &[]) }
    pub fn build_with(mix: &Mix, audio: &[Option<Arc<Audio>>], stems: &[Option<Arc<Stems>>]) -> Snap {
        let mut tracks = vec![]; let mut map = vec![usize::MAX; mix.tracks.len()];
        for (i, t) in mix.tracks.iter().enumerate() {
            if let Some(Some(a)) = audio.get(i) {
                map[i] = tracks.len();
                tracks.push(SnapTrack { stems: stems.get(i).cloned().flatten(), audio: a.clone(), lane: t.lane, start_beat: t.start_beat, offset: t.offset, bpm: t.bpm, vis: t.vis_start(mix.bpm), end: t.end(mix.bpm), grid: t.grid() });
            }
        }
        let trans = mix.transitions().into_iter().filter(|x| map[x.0] != usize::MAX && map[x.1] != usize::MAX)
            .map(|(a, b, s, e)| (map[a], map[b], s, e, mix.tracks[b].trans.preset, mix.tracks[b].trans.stems)).collect();
        Snap { key_lock: mix.key_lock, metro: if mix.metronome { Some(mix.metro_vol) } else { None }, bpm: mix.bpm, tracks, trans, decks: mix.decks, master: mix.master }
    }
}

pub enum Cmd { Snap(Arc<Snap>), Play(bool), Seek(f64), Loop(Option<(f64, f64)>) }

pub struct Shared { pub beat: AtomicU64, pub playing: AtomicBool, pub meter: [AtomicU32; 2], pub deck_meter: [AtomicU32; 4] }
impl Shared {
    pub fn new() -> Self { Shared { beat: AtomicU64::new(0f64.to_bits()), playing: AtomicBool::new(false), meter: Default::default(), deck_meter: Default::default() } }
    pub fn beat(&self) -> f64 { f64::from_bits(self.beat.load(Ordering::Relaxed)) }
    pub fn get(a: &AtomicU32) -> f32 { f32::from_bits(a.load(Ordering::Relaxed)) }
}

#[derive(Clone, Copy, Default)]
struct Biquad { b0: f32, b1: f32, b2: f32, a1: f32, a2: f32, z: [[f32; 2]; 2] }
impl Biquad {
    fn set(&mut self, kind: u8, f: f32, q: f32, gain_db: f32, sr: f32) {
        let w = 2.0 * PI * (f / sr).clamp(1e-4, 0.49); let (s, c) = w.sin_cos(); let al = s / (2.0 * q);
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            0 => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al), // LP
            1 => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al), // HP
            2 => { let sa = 2.0 * a.sqrt() * al; // low shelf
                (a * ((a + 1.0) - (a - 1.0) * c + sa), 2.0 * a * ((a - 1.0) - (a + 1.0) * c), a * ((a + 1.0) - (a - 1.0) * c - sa),
                 (a + 1.0) + (a - 1.0) * c + sa, -2.0 * ((a - 1.0) + (a + 1.0) * c), (a + 1.0) + (a - 1.0) * c - sa) }
            3 => { let sa = 2.0 * a.sqrt() * al; // high shelf
                (a * ((a + 1.0) + (a - 1.0) * c + sa), -2.0 * a * ((a - 1.0) + (a + 1.0) * c), a * ((a + 1.0) + (a - 1.0) * c - sa),
                 (a + 1.0) - (a - 1.0) * c + sa, 2.0 * ((a - 1.0) - (a + 1.0) * c), (a + 1.0) - (a - 1.0) * c - sa) }
            _ => (1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a), // peak
        };
        self.b0 = b0 / a0; self.b1 = b1 / a0; self.b2 = b2 / a0; self.a1 = a1 / a0; self.a2 = a2 / a0;
    }
    #[inline] fn run(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b0 * x + z[0]; z[0] = self.b1 * x - self.a1 * y + z[1]; z[1] = self.b2 * x - self.a2 * y; y
    }
}

/// Per-deck modulation computed from transitions at a given beat.
#[derive(Clone, Copy)]
struct Mods { gain: f32, low: f32, filter: f32, echo: f32, stem: Option<[f32; 4]> }
impl Default for Mods { fn default() -> Self { Mods { gain: 1.0, low: 1.0, filter: 0.0, echo: 0.0, stem: None } } }

fn smooth(x: f64) -> f32 { let x = x.clamp(0.0, 1.0) as f32; x * x * (3.0 - 2.0 * x) }

/// Modulation for one track at beat `b`: (track gain, low-gain, filter, echo-send)
fn track_mods(snap: &Snap, ti: usize, b: f64) -> Mods {
    let mut m = Mods::default();
    for &(o, i, s, e, p, sm) in &snap.trans {
        if ti != o && ti != i { continue; }
        let out = ti == o;
        let x = ((b - s) / (e - s)).clamp(0.0, 1.0);
        if out && b > e && p != Preset::EchoOut { m.gain = 0.0; continue; }
        if !out && b < s { m.gain = 0.0; continue; }
        if b < s || b > e { if out && p == Preset::EchoOut && b > e { m.gain = 0.0; } continue; }
        let cf_out = (x as f32 * PI / 2.0).cos(); let cf_in = (x as f32 * PI / 2.0).sin();
        if snap.tracks[o].stems.is_some() && snap.tracks[i].stems.is_some() {
            // real stems on both sides: the per-stem choices drive the gains, the preset keeps its EQ/filter/echo moves
            let g = [0, 1, 2, 3].map(|k| stem_gain(sm[k], x as f32, out));
            let prev = m.stem.unwrap_or([1.0; 4]); m.stem = Some([0, 1, 2, 3].map(|k| prev[k] * g[k]));
            match p {
                Preset::BassSwap => { let sw = smooth((x - 0.5) / 0.03 + 0.5); m.low *= if out { 1.0 - sw } else { sw }; }
                Preset::HighToLow => if out { m.filter = -0.95 * smooth(x / 0.9); },
                Preset::LowToHigh => if out { m.filter = 0.9 * smooth(x / 0.9); },
                Preset::EchoOut => if out { m.echo = if (0.35..0.5).contains(&x) { 1.0 } else { 0.0 }; },
                Preset::Crossfade => {}
            }
            continue;
        }
        match p {
            Preset::Crossfade => m.gain *= if out { cf_out } else { cf_in },
            Preset::BassSwap => {
                let sw = smooth((x - 0.5) / 0.03 + 0.5);
                if out { m.gain *= (1.0 - smooth((x - 0.5) * 2.0)).max(0.0); m.low *= 1.0 - sw; }
                else { m.gain *= smooth(x * 2.0); m.low *= sw; }
            }
            Preset::HighToLow => {
                if out { m.filter = -0.95 * smooth(x / 0.9); m.gain *= 1.0 - smooth((x - 0.8) / 0.2); } else { m.gain *= smooth(x / 0.6); }
            }
            Preset::LowToHigh => {
                if out { m.filter = 0.9 * smooth(x / 0.9); m.gain *= 1.0 - smooth((x - 0.8) / 0.2); } else { m.gain *= smooth(x / 0.6); }
            }
            Preset::EchoOut => {
                if out { m.echo = if (0.35..0.5).contains(&x) { 1.0 } else { 0.0 }; m.gain *= if x < 0.5 { 1.0 } else { 0.0 }; }
                else { m.gain *= smooth((x - 0.4) / 0.4); }
            }
        }
    }
    m
}

pub const WN: usize = 2048; // grain length (output samples)
pub const WH: usize = 512;  // synthesis hop (75% overlap)
const WTOL: i64 = 512;      // search tolerance (source samples)

/// Per-track WSOLA state (preallocated; no allocation in the audio thread).
pub struct Wsola { id: (usize, u64, u64, u64), acc: Vec<[f32; 2]>, fifo: Vec<[f32; 2]>, rd: usize, prev: Option<f64>, next_beat: f64, active: bool, win: Vec<f32> }
impl Wsola {
    pub fn new() -> Self { let win = (0..WN).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WN as f32).cos()).collect();
        Wsola { id: (0, 0, 0, 0), acc: vec![[0.0; 2]; WN], fifo: vec![[0.0; 2]; WH], rd: WH, prev: None, next_beat: f64::NAN, active: false, win } }
    fn reset(&mut self) { self.acc.iter_mut().for_each(|x| *x = [0.0; 2]); self.rd = WH; self.prev = None; self.active = false; }
    /// Produce one grain at mix beat `gb`; `pos_of` maps beat -> source sample position.
    fn grain(&mut self, a: &Audio, gb: f64, t: &SnapTrack, rs: f64, sg: Option<&[f32; 4]>) {
        let ideal = t.grid.sec(gb) * a.sr as f64; // beat map: stretch ratio follows tempo drift
        let get = |p: i64| -> f32 { if p >= 0 && (p as usize) < a.l.len() { a.l[p as usize] + a.r[p as usize] } else { 0.0 } };
        let mut start = ideal;
        if let Some(prev) = self.prev {
            let nat = prev + WH as f64 * rs; // natural continuation of the previous grain
            let (ni, ii) = (nat.round() as i64, ideal.round() as i64);
            let len = (WN / 2) as i64; let st = |rs: f64, j: i64| (j as f64 * rs) as i64;
            let score = |d: i64, step: usize| -> f32 { let mut c = 0f32; let mut e = 1e-9f32; let mut j = 0i64;
                while j < len { let x = get(ii + d + st(rs, j)); c += x * get(ni + st(rs, j)); e += x * x; j += step as i64; } c / e.sqrt() };
            let mut best = (f32::MIN, 0i64); let mut d = -WTOL;
            while d <= WTOL { let sc = score(d, 8); if sc > best.0 { best = (sc, d); } d += 16; }
            let c0 = best.1; let mut d = c0 - 16;
            while d <= c0 + 16 { let sc = score(d, 4); if sc > best.0 { best = (sc, d); } d += 2; }
            start = (ii + best.1) as f64 + (ideal - ii as f64);
        }
        for j in 0..WN {
            let p = start + j as f64 * rs; let k = p.floor() as i64; let f = (p - k as f64) as f32;
            if k < 0 || k as usize + 1 >= a.l.len() { continue; }
            let k = k as usize; let w = self.win[j] * 0.5;
            match (sg, t.stems.as_ref()) {
                (Some(g), Some(st)) => { for ch in 0..2 { self.acc[j][ch] += (st.mixed(a, ch, k, g) * (1.0 - f) + st.mixed(a, ch, k + 1, g) * f) * w; } }
                _ => { self.acc[j][0] += (a.l[k] * (1.0 - f) + a.l[k + 1] * f) * w; self.acc[j][1] += (a.r[k] * (1.0 - f) + a.r[k + 1] * f) * w; }
            }
        }
        self.prev = Some(start);
        self.fifo.copy_from_slice(&self.acc[..WH]);
        self.acc.copy_within(WH.., 0); for x in &mut self.acc[WN - WH..] { *x = [0.0; 2]; }
        self.rd = 0;
    }
}

struct DeckState { lowsh: Biquad, mid: Biquad, hish: Biquad, lpf: Biquad, hpf: Biquad, kill: Biquad, delay: Vec<[f32; 2]>, dpos: usize }

pub struct Engine {
    pub sr: f32, pub shared: Arc<Shared>, snap: Option<Arc<Snap>>, beat: f64, playing: bool, loop_r: Option<(f64, f64)>,
    decks: Vec<DeckState>, buf: Vec<[f32; 2]>, dbuf: [Vec<[f32; 2]>; 4], ws: Vec<Wsola>, click: (f32, f32), click_ph: f32, pub metro_enabled: bool,
}

impl Engine {
    pub fn new(sr: f32, shared: Arc<Shared>) -> Self {
        let decks = (0..4).map(|_| DeckState { lowsh: Biquad::default(), mid: Biquad::default(), hish: Biquad::default(), lpf: Biquad::default(), hpf: Biquad::default(), kill: Biquad::default(), delay: vec![[0.0; 2]; (sr * 3.0) as usize], dpos: 0 }).collect();
        Engine { sr, shared, snap: None, beat: 0.0, playing: false, loop_r: None, decks, buf: Vec::with_capacity(512), dbuf: Default::default(), ws: (0..64).map(|_| Wsola::new()).collect(), click: (0.0, 0.0), click_ph: 0.0, metro_enabled: true }
    }
    pub fn handle(&mut self, c: Cmd) {
        match c {
            Cmd::Snap(s) => self.snap = Some(s),
            Cmd::Play(p) => { self.playing = p; self.shared.playing.store(p, Ordering::Relaxed); }
            Cmd::Seek(b) => { self.beat = b; for w in &mut self.ws { w.reset(); } for d in &mut self.decks { d.delay.iter_mut().for_each(|x| *x = [0.0; 2]); } self.shared.beat.store(b.to_bits(), Ordering::Relaxed); }
            Cmd::Loop(l) => self.loop_r = l,
        }
    }
    /// Render interleaved output with `ch` channels.
    pub fn render(&mut self, out: &mut [f32], ch: usize) {
        let frames = out.len() / ch.max(1);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(256);
            self.block(n);
            for i in 0..n { let f = self.buf[i]; for c in 0..ch { out[(done + i) * ch + c] = if c % 2 == 0 { f[0] } else { f[1] }; } }
            done += n;
        }
    }
    fn block(&mut self, n: usize) {
        self.buf.clear(); self.buf.resize(n, [0.0; 2]); // capacity reserved in new()
        for d in &mut self.dbuf { d.clear(); d.resize(n, [0.0; 2]); }
        let Some(snap) = self.snap.clone() else { return; };
        if !self.playing { self.meters(); return; }
        let bpb = snap.bpm / 60.0 / self.sr as f64; // beats per output sample
        let b0 = self.beat; let b1 = b0 + bpb * n as f64;
        let mut deck_mod = [Mods::default(); 4];
        for (ti, t) in snap.tracks.iter().enumerate() {
            if b1 < t.vis || b0 > t.end + 8.0 { continue; }
            let m0 = track_mods(&snap, ti, b0); let m1 = track_mods(&snap, ti, b1);
            let dm = &mut deck_mod[t.lane];
            if b0 >= t.vis && b0 <= t.end { dm.low = m0.low; dm.filter = m0.filter; }
            dm.echo = dm.echo.max(m0.echo);
            if b0 > t.end { continue; }
            let a = &t.audio; let srt = a.sr as f64;
            let dst = &mut self.dbuf[t.lane];
            let rate = snap.bpm / t.bpm;
            if snap.key_lock && ((rate - 1.0).abs() > 1e-4 || t.grid.dynamic()) && ti < self.ws.len() {
                let w = &mut self.ws[ti];
                let id = (Arc::as_ptr(a) as usize, t.start_beat.to_bits(), t.offset.to_bits(), t.bpm.to_bits());
                if w.id != id || !w.active || (w.next_beat - b0).abs() > bpb * 2.0 {
                    w.reset(); w.id = id; w.active = true;
                    // pre-roll so the overlap-add is fully built up at b0
                    for k in (1..WN / WH).rev() { w.grain(a, b0 - (k * WH) as f64 * bpb, t, srt / self.sr as f64, m0.stem.as_ref()); }
                    w.next_beat = b0; w.rd = WH;
                }
                let rs = srt / self.sr as f64;
                for i in 0..n {
                    if w.rd >= WH { let gb = w.next_beat; w.grain(a, gb, t, rs, m0.stem.as_ref()); }
                    let f = w.fifo[w.rd]; w.rd += 1; w.next_beat += bpb;
                    let g = m0.gain + (m1.gain - m0.gain) * (i as f32 / n as f32);
                    dst[i][0] += f[0] * g; dst[i][1] += f[1] * g;
                }
                continue;
            }
            for i in 0..n {
                let b = b0 + bpb * i as f64;
                let sec = t.grid.sec(b);
                let pos = sec * srt;
                if pos < 1.0 || pos as usize + 2 >= a.l.len() { continue; }
                let g = m0.gain + (m1.gain - m0.gain) * (i as f32 / n as f32);
                if g <= 0.0 { continue; }
                let k = pos as usize; let f = (pos - k as f64) as f32;
                let sg = m0.stem; let st = t.stems.as_deref();
                let herm = |ch: usize| { let v = |i: usize| match (sg.as_ref(), st) { (Some(g), Some(st)) => st.mixed(a, ch, i, g), _ => if ch == 0 { a.l[i] } else { a.r[i] } };
                    let (y0, y1, y2, y3) = (v(k - 1), v(k), v(k + 1), v(k + 2));
                    let c1 = 0.5 * (y2 - y0); let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3; let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
                    ((c3 * f + c2) * f + c1) * f + y1 };
                dst[i][0] += herm(0) * g; dst[i][1] += herm(1) * g;
            }
        }
        let any_solo = snap.decks.iter().any(|d| d.solo);
        let sr = self.sr;
        let dly = ((0.75 * 60.0 / snap.bpm) as f32 * sr) as usize;
        for d in 0..4 {
            let p = snap.decks[d]; let m = deck_mod[d]; let st = &mut self.decks[d];
            let lowdb = if m.low < 0.01 { -40.0 } else { 20.0 * m.low.log10() };
            st.lowsh.set(2, 250.0, 0.7, (p.low * 12.0 + lowdb).max(-40.0), sr);
            st.mid.set(4, 1000.0, 0.7, p.mid * 12.0, sr);
            st.hish.set(3, 3000.0, 0.7, p.hi * 12.0, sr);
            st.kill.set(1, 30.0, 0.7, 0.0, sr);
            let filt = (p.filter + m.filter).clamp(-1.0, 1.0);
            let lpf = if filt < 0.0 { 20000.0 * (1.0 + filt).max(0.0).powf(3.0) + 80.0 } else { 20000.0 };
            let hpf = if filt > 0.0 { 20.0 + 8000.0 * filt.powf(2.5) } else { 20.0 };
            st.lpf.set(0, lpf.min(20000.0), 0.9, 0.0, sr); st.hpf.set(1, hpf, 0.9, 0.0, sr);
            let audible = !p.mute && (!any_solo || p.solo);
            let vol = if audible { p.vol } else { 0.0 };
            let mut pk = 0f32;
            let dlen = st.delay.len(); let dly = dly.min(dlen - 1).max(1);
            for i in 0..n {
                let mut y = [0f32; 2];
                for c in 0..2 {
                    let mut x = self.dbuf[d][i][c];
                    x = st.lowsh.run(c, x); x = st.mid.run(c, x); x = st.hish.run(c, x);
                    if filt < -0.001 { x = st.lpf.run(c, x); }
                    if filt > 0.001 { x = st.hpf.run(c, x); }
                    y[c] = x;
                }
                // echo: feedback delay, send amount from transition
                let rp = (st.dpos + dlen - dly) % dlen;
                let e = st.delay[rp];
                st.delay[st.dpos] = [y[0] * m.echo + e[1] * 0.55, y[1] * m.echo + e[0] * 0.55];
                st.dpos = (st.dpos + 1) % dlen;
                for c in 0..2 { let v = (y[c] + e[c] * 0.8) * vol; self.buf[i][c] += v; pk = pk.max(v.abs()); }
            }
            let old = Shared::get(&self.shared.deck_meter[d]);
            self.shared.deck_meter[d].store(pk.max(old * 0.9).to_bits(), Ordering::Relaxed);
        }
        let mut pk = [0f32; 2];
        for f in &mut self.buf { for c in 0..2 { let x = f[c] * snap.master; let y = if x.abs() < 0.9 { x } else { x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) * 10.0).tanh()) }; f[c] = y; pk[c] = pk[c].max(y.abs()); } }
        if let (Some(vol), true) = (snap.metro, self.metro_enabled) {
            for i in 0..n { let b = b0 + bpb * i as f64; let bn = b + bpb;
                if b.floor() != bn.floor() || (i == 0 && b.fract() < bpb) { let down = (bn.floor() as i64).rem_euclid(4) == 0; self.click = (if down { 1.0 } else { 0.55 }, if down { 1600.0 } else { 1000.0 }); self.click_ph = 0.0; }
                if self.click.0 > 1e-4 { self.click_ph += self.click.1 / self.sr; let v = (self.click_ph * 2.0 * PI).sin() * self.click.0 * vol * 0.5;
                    self.click.0 *= 0.9985; self.buf[i][0] += v; self.buf[i][1] += v; } }
        }
        for c in 0..2 { let old = Shared::get(&self.shared.meter[c]); self.shared.meter[c].store(pk[c].max(old * 0.92).to_bits(), Ordering::Relaxed); }
        self.beat = b1;
        if let Some((s, e)) = self.loop_r { if self.beat >= e { self.beat = s; } }
        self.shared.beat.store(self.beat.to_bits(), Ordering::Relaxed);
    }
    fn meters(&self) { for m in self.shared.meter.iter().chain(self.shared.deck_meter.iter()) { let v = Shared::get(m) * 0.9; m.store(v.to_bits(), Ordering::Relaxed); } }
}

/// Offline render of a whole mix, stereo.
pub fn render_offline(snap: Arc<Snap>, sr: u32, from: f64, to: f64) -> (Vec<f32>, Vec<f32>) {
    let mut e = Engine::new(sr as f32, Arc::new(Shared::new()));
    e.metro_enabled = false; // metronome is never exported
    e.handle(Cmd::Snap(snap.clone())); e.handle(Cmd::Seek(from)); e.handle(Cmd::Play(true));
    let frames = ((to - from) * 60.0 / snap.bpm * sr as f64) as usize;
    let mut buf = vec![0f32; 1024]; let (mut l, mut r) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    while l.len() < frames { e.render(&mut buf, 2); for f in buf.chunks(2) { l.push(f[0]); r.push(f[1]); } }
    l.truncate(frames); r.truncate(frames); (l, r)
}

pub fn write_wav(path: &std::path::Path, l: &[f32], r: &[f32], sr: u32) -> std::io::Result<()> {
    let n = l.len() as u32; let mut v = Vec::with_capacity(44 + n as usize * 4);
    v.extend(b"RIFF"); v.extend((36 + n * 4).to_le_bytes()); v.extend(b"WAVEfmt "); v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); v.extend(2u16.to_le_bytes()); v.extend(sr.to_le_bytes()); v.extend((sr * 4).to_le_bytes());
    v.extend(4u16.to_le_bytes()); v.extend(16u16.to_le_bytes()); v.extend(b"data"); v.extend((n * 4).to_le_bytes());
    for (a, b) in l.iter().zip(r) { for x in [a, b] { v.extend(((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); } }
    std::fs::write(path, v)
}

/// Volume curve of a preset at progress x (0..1) for the outgoing (`out`) or incoming track; used for drawing.
pub fn preset_gain(p: Preset, x: f32, out: bool) -> f32 {
    let xs = x as f64;
    match p {
        Preset::Crossfade => if out { (x * PI / 2.0).cos() } else { (x * PI / 2.0).sin() },
        Preset::BassSwap => if out { 1.0 - smooth((xs - 0.5) * 2.0) } else { smooth(xs * 2.0) },
        Preset::HighToLow | Preset::LowToHigh => if out { 1.0 - smooth((xs - 0.8) / 0.2) } else { smooth(xs / 0.6) },
        Preset::EchoOut => if out { if x < 0.5 { 1.0 } else { 0.0 } } else { smooth((xs - 0.4) / 0.4) },
    }
}

/// Per-stem gain in a transition at progress x for the outgoing (`out`) or incoming track.
/// None: stem silent in the overlap; Full: stem at full level; Crossfade: equal-power fade;
/// Swap: hard switch at the midpoint (short 3% ramp).
pub fn stem_gain(m: StemMode, x: f32, out: bool) -> f32 {
    match m {
        StemMode::None => 0.0,
        StemMode::Full => 1.0,
        StemMode::Crossfade => if out { (x * PI / 2.0).cos() } else { (x * PI / 2.0).sin() },
        StemMode::Swap => { let s = smooth(((x - 0.5) / 0.03 + 0.5) as f64); if out { 1.0 - s } else { s } }
    }
}
