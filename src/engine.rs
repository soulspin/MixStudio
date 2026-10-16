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

pub struct SnapTrack { pub stems: Option<Arc<Stems>>, pub audio: Arc<Audio>, pub lane: usize, pub start_beat: f64, pub offset: f64, pub bpm: f64, pub vis: f64, pub end: f64, pub grid: crate::beatmap::Grid, pub fade_in: f64, pub fade_out: f64, pub mutes: Vec<(u8, f64, f64)>, pub auto: Vec<Vec<(f64, f32)>>, pub stretch: u8 }
impl SnapTrack {
    /// clip gate and fades: 0 outside [vis, end), linear fades in beats
    #[inline] fn edge(&self, b: f64) -> f32 { if b < self.vis || b >= self.end { return 0.0; } let mut g = 1.0;
        if self.fade_in > 0.0 { g *= ((b - self.vis) / self.fade_in).min(1.0); } if self.fade_out > 0.0 { g *= ((self.end - b) / self.fade_out).min(1.0); } (g * self.grid.splice_gain(b)) as f32 }
}
#[allow(dead_code)]
pub struct Snap { pub key_lock: bool, pub metro: Option<f32>, pub bpm: f64, pub tracks: Vec<SnapTrack>, pub trans: Vec<(usize, usize, f64, f64, Preset, [StemMode; 4])>, pub decks: [Deck; 4], pub master: f32,
    pub tcurves: Vec<(Vec<(f32, f32)>, Vec<(f32, f32)>)>, pub fx: Vec<crate::fx::FxBlock>, pub live: Option<crate::fx::FxBlock>, pub tempo: Vec<(f64, f64)>,
    /// scratch blocks: (lane, start, end, parsed pattern, easing)
    pub scratch: Vec<(usize, f64, f64, Vec<Vec<crate::scratch::Key>>, f64)> }

impl Snap {
    pub fn build(mix: &Mix, audio: &[Option<Arc<Audio>>]) -> Snap { Self::build_with(mix, audio, &[]) }
    pub fn build_with(mix: &Mix, audio: &[Option<Arc<Audio>>], stems: &[Option<Arc<Stems>>]) -> Snap {
        let mut tracks = vec![]; let mut map = vec![usize::MAX; mix.tracks.len()];
        for (i, t) in mix.tracks.iter().enumerate() {
            if let Some(Some(a)) = audio.get(i) {
                map[i] = tracks.len();
                tracks.push(SnapTrack { stems: stems.get(i).cloned().flatten(), audio: a.clone(), lane: t.lane, start_beat: t.start_beat, offset: t.offset, bpm: t.eff_bpm(), vis: t.vis_start(mix.bpm), end: t.end(mix.bpm), grid: t.grid(), fade_in: t.fade_in, fade_out: t.fade_out, mutes: t.stem_mutes.iter().map(|m| (m.stem, m.a, m.b)).collect(), auto: t.auto.clone(), stretch: match t.stretch { 0 => if mix.key_lock { 1 } else { 2 }, x => x } });
            }
        }
        let tcurves = mix.transitions().into_iter().filter(|x| map[x.0] != usize::MAX && map[x.1] != usize::MAX).map(|x| (mix.tracks[x.1].trans.curve_out.clone(), mix.tracks[x.1].trans.curve_in.clone())).collect();
        let trans = mix.transitions().into_iter().filter(|x| map[x.0] != usize::MAX && map[x.1] != usize::MAX)
            .map(|(a, b, s, e)| (map[a], map[b], s, e, mix.tracks[b].trans.preset, mix.tracks[b].trans.stems)).collect();
        Snap { key_lock: mix.key_lock, metro: if mix.metronome { Some(mix.metro_vol) } else { None }, bpm: mix.bpm, tracks, trans, decks: mix.decks, master: mix.master, tcurves, fx: mix.fx.clone(), live: None, tempo: mix.tempo_curve(), scratch: mix.fx.iter().filter(|f| f.kind == crate::fx::FxKind::Scratch).filter_map(|f| crate::scratch::parse(&f.pattern).ok().map(|p| (f.target, f.start, f.start + f.len, p, f.ease as f64))).collect() }
    }
}

/// Producer-side capacity reconciliation: only new voices/effects allocate DSP buffers.
pub struct SnapshotBuilder { voices: usize, effects: Vec<FxKey>, retired_tx: crossbeam_channel::Sender<RetiredState>, retired_rx: crossbeam_channel::Receiver<RetiredState>, pending: Arc<std::sync::atomic::AtomicUsize>, spare_ws: Vec<Option<Wsola>>, spare_fx: Vec<(FxKey,Option<crate::fx::FxState>)> }
struct RetirementPermit(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for RetirementPermit {fn drop(&mut self) {self.0.fetch_sub(1,Ordering::Relaxed);}}
struct RetiredState { _permit: RetirementPermit, _ws: Vec<Option<Wsola>>, _fx: Vec<(FxKey,Option<crate::fx::FxState>)>, _snap: Option<Arc<Snap>> }
impl Default for SnapshotBuilder {
    fn default()->Self { let (retired_tx,retired_rx)=crossbeam_channel::bounded(2048);Self {voices:0,effects:vec![],retired_tx,retired_rx,pending:Arc::new(std::sync::atomic::AtomicUsize::new(0)),spare_ws:vec![],spare_fx:vec![]} }
}
pub struct PreparedSnap { permit: RetirementPermit, retired: crossbeam_channel::Sender<RetiredState>, snap: Arc<Snap>, ws: Vec<Option<Wsola>>, fx: Vec<(FxKey, Option<crate::fx::FxState>)> }
type FxKey = (u64, crate::fx::FxKind, usize, usize);
impl SnapshotBuilder {
    pub fn prepare(&mut self, snap: Arc<Snap>, sr: f32) -> PreparedSnap {
        while let Ok(mut old)=self.retired_rx.try_recv() {
            old._ws.clear();old._fx.clear();
            if old._ws.capacity()>self.spare_ws.capacity() {self.spare_ws=old._ws;}
            if old._fx.capacity()>self.spare_fx.capacity() {self.spare_fx=old._fx;}
        }
        // A permit covers each in-flight command and retired allocation. Hence the
        // callback always owns a reserved queue slot and cannot overflow it.
        while self.pending.load(Ordering::Relaxed)>=2048 { let _=self.retired_rx.recv(); }
        self.pending.fetch_add(1,Ordering::Relaxed);
        let mut ws=std::mem::take(&mut self.spare_ws);ws.extend((0..snap.tracks.len()).map(|i| if i < self.voices { None } else { Some(Wsola::new()) }));
        let mut keys: Vec<_>=snap.fx.iter().enumerate().map(|(i,f)| {
            let occurrence=snap.fx[..i].iter().filter(|p|p.id==f.id && p.kind==f.kind && p.target==f.target).count();
            (f.id,f.kind,f.target,occurrence)
        }).collect();
        keys.push(snap.live.as_ref().map(|f|(f.id,f.kind,f.target,usize::MAX)).unwrap_or((0,crate::fx::FxKind::Scratch,4,usize::MAX)));
        let mut fx=std::mem::take(&mut self.spare_fx);fx.extend(keys.iter().map(|key| (*key, if self.effects.contains(key) { None } else { Some(crate::fx::FxState::new(sr)) })));
        self.voices=snap.tracks.len();self.effects=keys;
        PreparedSnap { permit:RetirementPermit(self.pending.clone()), retired:self.retired_tx.clone(), snap, ws, fx }
    }
}
pub enum Cmd { Snap(Arc<Snap>), Prepared(PreparedSnap), Play(bool), Seek(f64), Loop(Option<(f64, f64)>) }

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
    for (tk, &(o, i, s, e, p, sm)) in snap.trans.iter().enumerate() {
        if ti != o && ti != i { continue; }
        let out = ti == o;
        if snap.trans[..tk].iter().any(|&(po,pi,ps,pe,_,_)| ps==s && pe==e && if out {po==ti} else {pi==ti}) { continue; }
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
        let g_before = m.gain;
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
        if let Some(c) = snap.tcurves.get(tk).map(|c| if out { &c.0 } else { &c.1 }).filter(|c| !c.is_empty()) { m.gain = g_before * crate::model::interp32(c, x as f32); }
    }
    m
}

pub const WN: usize = 2048; // grain length (output samples)
pub const WH: usize = 512;  // synthesis hop (75% overlap)
const WTOL: i64 = 256;      // search tolerance (source samples); small so transients stay within ~6 ms of the beat map

/// Per-track WSOLA state (preallocated; no allocation in the audio thread).
pub struct Wsola { id: (usize, u64, u64, u64, u64), acc: Vec<[f32; 2]>, fifo: Vec<[f32; 2]>, rd: usize, prev: Option<f64>, next_beat: f64, active: bool, tiny_loop: Option<(u64,u64)>, win: Vec<f32> }
impl Wsola {
    pub fn new() -> Self { let win = (0..WN).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WN as f32).cos()).collect();
        Wsola { id: (0, 0, 0, 0, 0), acc: vec![[0.0; 2]; WN], fifo: vec![[0.0; 2]; WH], rd: WH, prev: None, next_beat: f64::NAN, active: false, tiny_loop: None, win } }
    fn reset(&mut self) { self.acc.iter_mut().for_each(|x| *x = [0.0; 2]); self.rd = WH; self.prev = None; self.active = false; self.tiny_loop=None; }
    /// Produce one grain at mix beat `gb`; `pos_of` maps beat -> source sample position.
    fn grain(&mut self, a: &Audio, gb: f64, at: f64, t: &SnapTrack, rs: f64, sg: Option<&[f32; 4]>) {
        let comb = combine(sg.copied(), t.mute_gain(t.grid.region_sec(gb, at))); let sg = comb.as_ref();
        let ideal = t.grid.region_sec(gb, at) * a.sr as f64; // beat map: stretch ratio follows tempo drift
        let get = |p: i64| -> f32 { if p >= 0 && (p as usize) < a.l.len() { a.l[p as usize] + a.r[p as usize] } else { 0.0 } };
        let mut start = ideal;
        if let Some(prev) = self.prev {
            let nat = prev + WH as f64 * rs; // natural continuation of the previous grain
            let (ni, ii) = (nat.round() as i64, ideal.round() as i64);
            let len = (WN / 2) as i64; let st = |rs: f64, j: i64| (j as f64 * rs) as i64;
            let score = |d: i64, step: usize| -> f32 { let mut c = 0f32; let mut e = 1e-9f32; let mut j = 0i64;
                while j < len { let x = get(ii + d + st(rs, j)); c += x * get(ni + st(rs, j)); e += x * x; j += step as i64; } c / e.sqrt() };
            // transient guard: if a sharp energy rise (kick/snare attack) falls inside this grain, place the grain
            // exactly at its beat-map position. A similarity shift here would move the attack by up to WTOL,
            // in a direction that depends on whether the track is sped up or slowed down.
            let blk = 256i64; let be = |p: i64| { let mut e = 1e-7f32; let mut j = 0; while j < blk { let x = get(p + j); e += x * x; j += 2; } e };
            let mut transient = false; let mut prev_e = be(ii - blk); let mut q = ii;
            while q < ii + WN as i64 { let e = be(q); if e > 4.0 * prev_e && e > 1e-4 * blk as f32 { transient = true; break; } prev_e = e; q += blk; }
            if transient { start = ideal; } else {
            let mut best = (f32::MIN, 0i64); let mut d = -WTOL;
            let pen = |d: i64, sc: f32| sc - 0.15 * sc.abs() * (d as f32 / WTOL as f32).powi(2); // prefer the ideal (beat-locked) position
            while d <= WTOL { let sc = pen(d, score(d, 8)); if sc > best.0 { best = (sc, d); } d += 16; }
            let c0 = best.1; let mut d = c0 - 16;
            while d <= c0 + 16 { let sc = pen(d, score(d, 4)); if sc > best.0 { best = (sc, d); } d += 2; }
            start = (ii + best.1) as f64 + (ideal - ii as f64);
            }
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

struct DeckState { settings: [f32;4], lowsh: Biquad, mid: Biquad, hish: Biquad, lpf: Biquad, hpf: Biquad, kill: Biquad, delay: Vec<[f32; 2]>, dpos: usize }

pub struct Engine {
    pub sr: f32, pub shared: Arc<Shared>, snap: Option<Arc<Snap>>, beat: f64, playing: bool, loop_r: Option<(f64, f64)>,
    decks: Vec<DeckState>, buf: Vec<[f32; 2]>, dbuf: [Vec<[f32; 2]>; 4], ws: Vec<Option<Wsola>>, click: (f32, f32), click_ph: f32, click_beat: Option<i64>, pub metro_enabled: bool, fx: Vec<(FxKey, Option<crate::fx::FxState>)>,
}

impl Engine {
    pub fn new(sr: f32, shared: Arc<Shared>) -> Self {
        let decks = (0..4).map(|_| DeckState { settings:[f32::NAN;4], lowsh: Biquad::default(), mid: Biquad::default(), hish: Biquad::default(), lpf: Biquad::default(), hpf: Biquad::default(), kill: Biquad::default(), delay: vec![[0.0; 2]; ((sr * 3.0) as usize).max(2)], dpos: 0 }).collect();
        Engine { sr, shared, snap: None, beat: 0.0, playing: false, loop_r: None, decks, buf: Vec::with_capacity(512), dbuf: std::array::from_fn(|_| Vec::with_capacity(256)), ws: vec![], click: (0.0, 0.0), click_ph: 0.0, click_beat: None, metro_enabled: true, fx: vec![] }
    }
    pub fn handle(&mut self, c: Cmd) {
        match c {
            Cmd::Snap(s) => {
                // Offline/test convenience. Realtime callers prepare on the producer thread.
                let mut builder=SnapshotBuilder { voices:self.ws.len(), effects:self.fx.iter().map(|f|f.0).collect(), ..Default::default() };
                self.install(builder.prepare(s,self.sr));
            }
            Cmd::Prepared(p) => self.install(p),
            Cmd::Play(p) => { self.playing = p; self.shared.playing.store(p, Ordering::Relaxed); }
            Cmd::Seek(b) => { self.beat = b; self.reset_transport(); self.shared.beat.store(b.to_bits(), Ordering::Relaxed); }
            Cmd::Loop(l) => self.loop_r = l,
        }
    }
    fn install(&mut self, mut p: PreparedSnap) {
        for (i,w) in p.ws.iter_mut().enumerate() { if w.is_none() { *w=self.ws.get_mut(i).and_then(Option::take); } }
        for (key,state) in &mut p.fx { if state.is_none() { if let Some(old)=self.fx.iter_mut().find(|f|f.0==*key) { *state=old.1.take(); } } }
        if let Some(old)=&self.snap {
            let envelope_changed=old.tempo!=p.snap.tempo || old.bpm!=p.snap.bpm || old.trans!=p.snap.trans || old.tcurves!=p.snap.tcurves;
            for (i,w) in p.ws.iter_mut().enumerate() {
                if let Some(w)=w.as_mut().filter(|w|w.tiny_loop.is_some()) {
                    let source_changed=old.tracks.get(i).zip(p.snap.tracks.get(i)).is_none_or(|(a,b)|a.mutes!=b.mutes || match (&a.stems,&b.stems) {(Some(a),Some(b))=>!Arc::ptr_eq(a,b),(None,None)=>false,_=>true});
                    if envelope_changed || source_changed {w.tiny_loop=None;w.active=false;}
                }
            }
        }
        let retired=RetiredState {_permit:p.permit,_ws:std::mem::replace(&mut self.ws,p.ws),_fx:std::mem::replace(&mut self.fx,p.fx),_snap:self.snap.replace(p.snap)};
        // Reclaim removed DSP buffers on the snapshot producer, away from the callback.
        match p.retired.try_send(retired) {
            Ok(()) | Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}, // offline producer is short-lived
            Err(crossbeam_channel::TrySendError::Full(_)) => unreachable!("retirement queue slot reserved by permit"),
        }
    }
    fn reset_transport(&mut self) {
        for w in self.ws.iter_mut().flatten() { w.reset(); }
        for d in &mut self.decks {
            d.lowsh.z=[[0.0;2];2];d.mid.z=[[0.0;2];2];d.hish.z=[[0.0;2];2];d.lpf.z=[[0.0;2];2];d.hpf.z=[[0.0;2];2];d.kill.z=[[0.0;2];2];
            d.delay.fill([0.0;2]);d.dpos=0;
        }
        for f in &mut self.fx { if let Some(f)=&mut f.1 {f.reset();} }
        self.click=(0.0,0.0);self.click_ph=0.0;self.click_beat=None;
    }
    /// Render interleaved output with `ch` channels.
    pub fn render(&mut self, out: &mut [f32], ch: usize) {
        let frames = out.len() / ch.max(1);
        let mut done = 0;
        while done < frames {
            let mut n = (frames - done).min(256);
            if let (Some((s,e)),Some(snap))=(self.loop_r,self.snap.as_ref()) {
                if e>s && self.playing {
                    if self.beat>=e { self.beat=snap.advance(s,snap.secs(e,self.beat).rem_euclid(snap.secs(s,e))); }
                    n=n.min((snap.secs(self.beat,e)*self.sr as f64).ceil().max(1.0) as usize);
                }
            }
            if let Some(snap)=self.snap.as_ref() {
                for s in &snap.scratch {for boundary in [s.1,s.2] {if boundary>self.beat {
                    n=n.min((snap.secs(self.beat,boundary)*self.sr as f64).ceil().max(1.0) as usize);
                }}}
            }
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
        let bpm_now = snap.bpm_at(self.beat);
        let bpb = bpm_now / 60.0 / self.sr as f64; // beats per output sample (tempo curve, per block)
        let b0 = self.beat;
        let mut beats=[0.0;257];beats[0]=b0;
        for i in 0..n { beats[i+1]=snap.advance(beats[i],1.0/self.sr as f64); }
        let b1=beats[n];
        let short_loop=self.loop_r.filter(|&(s,e)|e>s && snap.secs(s,e)*self.sr as f64<=WH as f64);
        let mut deck_mod = [[Mods::default();256];4]; let mut deck_auto = [[[0f32;4];256];4];
        for (ti, t) in snap.tracks.iter().enumerate() {
            if b1 < t.vis || b0 > t.end + 8.0 { continue; }
            for i in 0..n {
                let b=beats[i];let m=track_mods(&snap,ti,b);let dm=&mut deck_mod[t.lane][i];
                if b>=t.vis && b<t.end {dm.low=m.low;dm.filter=m.filter;for k in 0..4 {if let Some(v)=t.auto_at(k+1,b) {deck_auto[t.lane][i][k]=v;}}}
                dm.echo=dm.echo.max(m.echo);
            }
            if b0 > t.end { continue; }
            let a = &t.audio; let srt = a.sr as f64;
            let dst = &mut self.dbuf[t.lane];
            let rate = bpm_now / t.bpm;
            let scr = snap.scratch.iter().find(|s| s.0 == t.lane && b0 >= s.1 && b0 < s.2);
            if scr.is_none() && t.stretch == 1 && ((rate - 1.0).abs() > 1e-4 || t.grid.dynamic() || snap.tempo.iter().any(|p|(p.1-t.bpm).abs()>1e-4)) && ti < self.ws.len() {
                let w = self.ws[ti].as_mut().expect("prepared voice state");
                let rs = srt / self.sr as f64;
                for i in 0..n {
                    let b = beats[i];
                    if b<t.vis || b>=t.end {w.active=false;continue;}
                    let id = (Arc::as_ptr(a) as usize, t.start_beat.to_bits(), t.offset.to_bits(), t.bpm.to_bits(), t.grid.section_id(b));
                    if let Some((ls,le))=short_loop.filter(|&(s,e)|b>=s && b<e) {
                        let key=(ls.to_bits(),le.to_bits());
                        if w.tiny_loop!=Some(key) || w.id!=id || !w.active {
                            w.reset();w.id=id;w.active=true;w.tiny_loop=Some(key);
                            for k in (1..WN/WH).rev() {w.grain(a,snap.advance(ls,-((k*WH) as f64)/self.sr as f64),ls,t,rs,track_mods(&snap,ti,ls).stem.as_ref());}
                            w.grain(a,ls,ls,t,rs,track_mods(&snap,ti,ls).stem.as_ref());
                        }
                        // Sub-hop loops read the same anchored grain at their exact
                        // time phase. Regenerating four grains on every one-sample
                        // wrap would be both wasteful and callback dependent.
                        let phase=(snap.secs(ls,b)*self.sr as f64).clamp(0.0,WH as f64-f64::EPSILON);
                        let index=(phase as usize).min(WH-1);let fraction=(phase-index as f64) as f32;
                        let next=if index+1<WH {w.fifo[index+1]} else {w.acc[0]};
                        let g=track_mods(&snap,ti,b).gain*t.auto_at(0,b).unwrap_or(1.0)*t.edge(b);
                        for c in 0..2 {dst[i][c]+=(w.fifo[index][c]*(1.0-fraction)+next[c]*fraction)*g;}
                        w.rd=index;w.next_beat=beats[i+1];
                        continue;
                    }
                    if w.tiny_loop.is_some() {w.reset();}
                    if w.id != id || !w.active || (w.next_beat-b).abs()>bpb*2.0 {
                        w.reset(); w.id=id; w.active=true;
                        // Pre-roll within this source span, including immediately after a splice.
                        for k in (1..WN/WH).rev() { w.grain(a,snap.advance(b,-((k*WH) as f64)/self.sr as f64),b,t,rs,track_mods(&snap,ti,b).stem.as_ref()); }
                        w.next_beat=b; w.rd=WH;
                    }
                    if w.rd >= WH { let gb=w.next_beat; w.grain(a,gb,b,t,rs,track_mods(&snap,ti,b).stem.as_ref()); }
                    let f = w.fifo[w.rd]; w.rd += 1; w.next_beat = beats[i+1];
                    let g = (track_mods(&snap,ti,beats[i]).gain * t.auto_at(0,beats[i]).unwrap_or(1.0)) * t.edge(beats[i]);
                    dst[i][0] += f[0] * g; dst[i][1] += f[1] * g;
                }
                continue;
            }
            for i in 0..n {
                let b = beats[i];
                let (bb, scg) = match scr { Some(s) if b >= s.1 && b < s.2 => { let (x, g) = crate::scratch::eval(&s.3, b - s.1, s.4); (s.1 + x, g) } _ => (b, 1.0) };
                let (sec, slg) = if t.stretch == 3 && bb == b { t.slice(b, snap.bpm_at(b)) } else { (t.grid.sec(bb), 1.0) };
                let pos = sec * srt;
                if pos < 1.0 || pos as usize + 2 >= a.l.len() { continue; }
                let g = (track_mods(&snap,ti,beats[i]).gain * t.auto_at(0,beats[i]).unwrap_or(1.0)) * t.edge(b) * slg * scg;
                if g <= 0.0 { continue; }
                let k = pos as usize; let f = (pos - k as f64) as f32;
                let sg = combine(track_mods(&snap,ti,b).stem, t.mute_gain(sec)); let st = t.stems.as_deref();
                let herm = |ch: usize| { let v = |i: usize| match (sg.as_ref(), st) { (Some(g), Some(st)) => st.mixed(a, ch, i, g), _ => if ch == 0 { a.l[i] } else { a.r[i] } };
                    let (y0, y1, y2, y3) = (v(k - 1), v(k), v(k + 1), v(k + 2));
                    let c1 = 0.5 * (y2 - y0); let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3; let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
                    ((c3 * f + c2) * f + c1) * f + y1 };
                dst[i][0] += herm(0) * g; dst[i][1] += herm(1) * g;
            }
        }
        for (k, f) in snap.fx.iter().enumerate() { if f.target < 4 { let (st, buf) = (self.fx[k].1.as_mut().expect("prepared effect state"), &mut self.dbuf[f.target]); st.process(f, buf, &beats[..=n], &snap, false); } }
        let any_solo = snap.decks.iter().any(|d| d.solo);
        let sr = self.sr;
        for d in 0..4 {
            let p = snap.decks[d]; let st = &mut self.decks[d];
            let audible = !p.mute && (!any_solo || p.solo);
            let vol = if audible { p.vol } else { 0.0 };
            let mut pk = 0f32;
            let dlen = st.delay.len();
            for i in 0..n {
                let m=deck_mod[d][i];let aut=deck_auto[d][i];
                let lowdb=if m.low<0.01 {-40.0} else {20.0*m.low.log10()};
                let filt=(p.filter+m.filter+aut[3]).clamp(-1.0,1.0);
                let settings=[((p.low+aut[0]).clamp(-1.0,1.0)*12.0+lowdb).max(-40.0),(p.mid+aut[1]).clamp(-1.0,1.0)*12.0,(p.hi+aut[2]).clamp(-1.0,1.0)*12.0,filt];
                if st.settings!=settings {
                    st.lowsh.set(2,250.0,0.7,settings[0],sr);st.mid.set(4,1000.0,0.7,settings[1],sr);st.hish.set(3,3000.0,0.7,settings[2],sr);
                    let lpf=if filt<0.0 {20000.0*(1.0+filt).max(0.0).powf(3.0)+80.0} else {20000.0};
                    let hpf=if filt>0.0 {20.0+8000.0*filt.powf(2.5)} else {20.0};
                    st.lpf.set(0,lpf.min(20000.0),0.9,0.0,sr);st.hpf.set(1,hpf,0.9,0.0,sr);st.settings=settings;
                }
                let dly= ((0.75*60.0/snap.bpm_at(beats[i]))*sr as f64) as usize;
                let dly=dly.min(dlen-1).max(1);
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
        for (k, f) in snap.fx.iter().enumerate() { if f.target >= 4 { self.fx[k].1.as_mut().expect("prepared effect state").process(f, &mut self.buf, &beats[..=n], &snap, false); } }
        { let last = self.fx.len() - 1; match &snap.live { Some(f) => self.fx[last].1.as_mut().expect("prepared live state").process(f, &mut self.buf, &beats[..=n], &snap, true), None => if self.fx[last].1.as_ref().is_some_and(|f|f.active) { self.fx[last].1.as_mut().unwrap().reset(); } } }
        let mut pk = [0f32; 2];
        for f in &mut self.buf { for c in 0..2 { let x = f[c] * snap.master; let y = if x.abs() < 0.9 { x } else { x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) * 10.0).tanh()) }; f[c] = y; pk[c] = pk[c].max(y.abs()); } }
        if let (Some(vol), true) = (snap.metro, self.metro_enabled) {
            for i in 0..n { let b = beats[i];
                if self.click_beat != Some(b.floor() as i64) { self.click_beat=Some(b.floor() as i64);let down = (b.floor() as i64).rem_euclid(4) == 0; self.click = (if down { 1.0 } else { 0.55 }, if down { 1600.0 } else { 1000.0 }); self.click_ph = 0.0; }
                if self.click.0 > 1e-4 { self.click_ph += self.click.1 / self.sr; let v = (self.click_ph * 2.0 * PI).sin() * self.click.0 * vol * 0.5;
                    self.click.0 *= 0.9985; self.buf[i][0] += v; self.buf[i][1] += v; } }
        }
        for c in 0..2 { let old = Shared::get(&self.shared.meter[c]); self.shared.meter[c].store(pk[c].max(old * 0.92).to_bits(), Ordering::Relaxed); }
        self.beat = b1;
        if let Some((s,e))=self.loop_r { if e>s && self.beat>=e {
            let duration=snap.secs(s,e);let extra=snap.secs(e,self.beat);
            self.beat=snap.advance(s,extra.rem_euclid(duration));
            for w in self.ws.iter_mut().flatten() {
                if w.tiny_loop.is_some() {w.rd=0;} else {w.active=false;}
            }
        }}
        self.shared.beat.store(self.beat.to_bits(), Ordering::Relaxed);
    }
    fn meters(&self) { for m in self.shared.meter.iter().chain(self.shared.deck_meter.iter()) { let v = Shared::get(m) * 0.9; m.store(v.to_bits(), Ordering::Relaxed); } }
}

/// Offline render of a whole mix, stereo.
pub fn render_offline(snap: Arc<Snap>, sr: u32, from: f64, to: f64) -> (Vec<f32>, Vec<f32>) {
    let mut e = Engine::new(sr as f32, Arc::new(Shared::new()));
    e.metro_enabled = false; // metronome is never exported
    e.handle(Cmd::Snap(snap.clone())); e.handle(Cmd::Seek(from)); e.handle(Cmd::Play(true));
    let frames = (snap.secs(from, to) * sr as f64).round() as usize;
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
impl SnapTrack {
    /// stem gains from muted ranges at source second `sec` (5 ms ramps); None when nothing is muted
    #[inline] pub fn mute_gain(&self, sec: f64) -> Option<[f32; 4]> {
        if self.mutes.is_empty() || self.stems.is_none() { return None; }
        let mut g = [1.0f32; 4]; let mut any = false;
        for &(k, a, b) in &self.mutes { if sec > a - 0.005 && sec < b + 0.005 {
            let f = ((sec - a + 0.005) / 0.01).min((b + 0.005 - sec) / 0.01).clamp(0.0, 1.0) as f32; g[k as usize & 3] *= 1.0 - f; any = true; } }
        if any { Some(g) } else { None }
    }
}
#[inline] fn combine(a: Option<[f32; 4]>, b: Option<[f32; 4]>) -> Option<[f32; 4]> {
    match (a, b) { (Some(x), Some(y)) => Some([x[0] * y[0], x[1] * y[1], x[2] * y[2], x[3] * y[3]]), (x, None) => x, (None, y) => y }
}
impl SnapTrack {
    #[inline] pub fn auto_at(&self, k: usize, b: f64) -> Option<f32> { self.auto.get(k).and_then(|p| crate::model::interp(p, b - self.start_beat)) }
}
impl Snap {
    #[inline] pub fn bpm_at(&self, b: f64) -> f64 { crate::model::bpm_on(&self.tempo, self.bpm, b) }
    pub fn advance(&self, mut b: f64, mut seconds: f64) -> f64 {
        let forward=seconds>=0.0;
        while seconds.abs()>1e-15 {
            let mut node=if forward {self.tempo.iter().find(|p|p.0>b).map(|p|p.0)} else {self.tempo.iter().rev().find(|p|p.0<b).map(|p|p.0)};
            // bpm_on clamps curves at 20 BPM: the crossing is another integration
            // boundary even though it is not a user-authored tempo node.
            for pair in self.tempo.windows(2) {
                let (p,q)=(pair[0],pair[1]);
                if (p.1<20.0 && q.1>20.0)||(p.1>20.0 && q.1<20.0) {
                    let crossing=p.0+(20.0-p.1)*(q.0-p.0)/(q.1-p.1);
                    if if forward {crossing>b && node.is_none_or(|n|crossing<n)} else {crossing<b && node.is_none_or(|n|crossing>n)} {node=Some(crossing);}
                }
            }
            let bpm=self.bpm_at(b);
            let Some(end)=node else {return b+seconds*bpm/60.0;};
            let end_bpm=self.bpm_at(end);let slope=(end_bpm-bpm)/(end-b);
            let span=if slope.abs()<1e-12 {60.0*(end-b)/bpm} else {60.0*((end_bpm-bpm)/bpm).ln_1p()/slope};
            if seconds.abs()<=span.abs() {
                return b+if slope.abs()<1e-12 {seconds*bpm/60.0} else {bpm*(seconds*slope/60.0).exp_m1()/slope};
            }
            b=end;seconds-=span;
        }
        b
    }
    pub fn secs(&self, a: f64, b: f64) -> f64 { crate::model::secs_on(&self.tempo, self.bpm, a, b) }
}
impl SnapTrack {
    /// Beat slice: each beat's audio plays at its original speed from the beat start; it is cut at
    /// the next mix beat (faster tempo) or ends early leaving a short gap (slower tempo). 2 ms fades.
    #[inline] fn slice(&self, b: f64, bpm: f64) -> (f64, f32) {
        let k = (b - self.start_beat).floor(); let bs = self.start_beat + k; let s0 = self.grid.region_sec(bs,b); let s1 = self.grid.region_sec(bs + 1.0,b);
        let el = (b - bs) * 60.0 / bpm; let sec = s0 + el; let left = (1.0 - (b - bs)) * 60.0 / bpm;
        let g = (el / 0.002).min((s1 - sec) / 0.002).min(left / 0.002).clamp(0.0, 1.0) as f32; (sec, g)
    }
}

#[cfg(test)]
mod section_audio_tests {
    use super::*;
    #[test]
    fn beat_slice_does_not_silence_the_beat_before_a_repeat() {
        let mut m=Mix::default();
        let t: crate::model::Track=serde_json::from_str(r#"{"path":"test","title":"Test","artist":"Test","bpm":120.0,"offset":0.0,"key":"8A","duration":32.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap();
        m.tracks.push(t); let c=m.tracks[0].copy_section(8.0,16.0).unwrap(); m.insert_section(0,16.0,&c).unwrap();
        let snap=Snap::build(&m,&[Some(Arc::new(Audio {sr:8000,l:vec![0.25;8000*32],r:vec![0.25;8000*32]}))]);
        let (sec,gain)=snap.tracks[0].slice(15.5,120.0); assert!((sec-7.75).abs()<1e-8); assert!(gain>0.99);
        let c=m.tracks[0].copy_section(8.25,10.75).unwrap(); m.insert_section(0,20.25,&c).unwrap();
        let snap=Snap::build(&m,&[Some(Arc::new(Audio {sr:8000,l:vec![0.25;8000*32],r:vec![0.25;8000*32]}))]);
        let (sec,gain)=snap.tracks[0].slice(20.5,120.0); assert!((sec-4.25).abs()<1e-8); assert!(gain>0.99);
    }
    #[test]
    fn section_splices_have_short_fades_and_stable_render_timing() {
        let t=serde_json::from_str(r#"{"path":"test","title":"Test","artist":"Test","bpm":120.0,"offset":0.0,"key":"8A","duration":8.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap();
        let mut mix=Mix::default(); mix.bpm=120.0; mix.tracks.push(t);
        mix.delete_section(0,4.0,8.0).unwrap();
        let sr=16000;
        let signal: Vec<_>=(0..sr*8).map(|i| { let f=if i<sr*4 { 220.0 } else { 440.0 }; 0.25*(2.0*PI*f*i as f32/sr as f32).sin() }).collect();
        let audio=Arc::new(Audio { sr, l:signal.clone(), r:signal });
        for stretch in [1,2,3] {
            mix.tracks[0].stretch=stretch; mix.bpm=126.0;
            let snap=Arc::new(Snap::build(&mix,&[Some(audio.clone())]));
            assert!(snap.tracks[0].edge(4.0)<0.001,"splice must fade at its boundary");
            assert!(snap.tracks[0].edge(4.1)>0.99,"fade must stay short");
            let (l,r)=render_offline(snap,sr,0.0,12.0);
            assert_eq!(l.len(),r.len()); assert!(l.iter().all(|v| v.is_finite() && v.abs()<=1.0));
            let (at, before)=((4.0*60.0/126.0*sr as f64) as usize,(3.8*60.0/126.0*sr as f64) as usize);
            assert!(l[before..at+160].windows(2).all(|w| (w[1]-w[0]).abs()<0.3),"audible discontinuity, stretch {stretch}");
        }
    }
}

#[cfg(test)]
mod saved_mix_diagnostics {
    use super::*;
    #[test]
    #[ignore = "requires MIXSTUDIO_DIAGNOSTIC_PROJECT pointing to local audio"]
    fn compare_static_and_live_snapshots_in_saved_transition() {
        let path=std::env::var("MIXSTUDIO_DIAGNOSTIC_PROJECT").unwrap();
        let mix:Mix=serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let audio:Vec<_>=mix.tracks.iter().map(|t|Some(Arc::new(crate::analysis::load(std::path::Path::new(&t.path)).unwrap().0))).collect();
        let (_,_,s,e)=mix.transitions()[0];
        println!("Transition {s:.3}..{e:.3}, {:.2}s, mix BPM {} mode {}",mix.secs(s,e),mix.bpm,mix.tempo_mode);
        for mode in [1,0] {
            let mut compare=mix.clone(); for t in &mut compare.tracks { t.grid_mode=mode; }
            println!("Alignment grid {mode}: {:?}",crate::overlap_alignment(&compare,&audio,44100,true));
        }
        for sr in [44100,48000] {
            let snap=Arc::new(Snap::build(&mix,&audio));
            let t0=std::time::Instant::now(); let (l,r)=render_offline(snap.clone(),sr,s-8.0,e+8.0);
            println!("Static {sr}: {:.3}s CPU for {:.3}s audio",t0.elapsed().as_secs_f64(),l.len() as f64/sr as f64);
            let mut engine=Engine::new(sr as f32,Arc::new(Shared::new()));engine.handle(Cmd::Snap(snap));engine.handle(Cmd::Seek(s-8.0));engine.handle(Cmd::Play(true));
            let mut buf=vec![0.0;1024];let mut rendered=0;let mut maxdiff=0.0f32; let mut maxcost=0.0f64; let mut ids=vec![];let t0=std::time::Instant::now();
            while rendered<l.len() {
                if rendered % (512*4)==0 { engine.handle(Cmd::Snap(Arc::new(Snap::build(&mix.clone(),&audio)))); }
                let start=std::time::Instant::now();engine.render(&mut buf,2);maxcost=maxcost.max(start.elapsed().as_secs_f64());
                let next:Vec<_>=engine.ws.iter().take(2).map(|w|w.as_ref().unwrap().id).collect();if ids!=next {println!("WSOLA identity changed at beat {:.3}: {:?}",engine.beat,next);ids=next;}
                for i in 0..512.min(l.len()-rendered) {maxdiff=maxdiff.max((buf[i*2]-l[rendered+i]).abs()).max((buf[i*2+1]-r[rendered+i]).abs());} rendered+=512;
            }
            println!("Live {sr}: {:.3}s CPU, max callback {:.3}ms, static/live diff {maxdiff}",t0.elapsed().as_secs_f64(),maxcost*1000.0);
            assert!(maxdiff<1e-6);
            let dir=std::env::temp_dir().join("mixstudio-current-audio-check");std::fs::create_dir_all(&dir).unwrap();
            write_wav(&dir.join(format!("current-{sr}.wav")),&l,&r,sr).unwrap();
        }
    }
}

#[cfg(test)]
mod maintenance_tests {
    use super::*;
    fn snap() -> Snap { Snap::build(&Mix::default(), &[]) }
    fn tone(s: &mut Snap) {
        let mut mix=Mix::default(); mix.bpm=126.0;
        let t=serde_json::from_str(r#"{"path":"synthetic","title":"tone","artist":"test","bpm":120.0,"offset":0.0,"key":"8A","duration":8.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap(); mix.tracks.push(t);
        let v=(0..64000).map(|i|0.2*(2.0*PI*440.0*i as f32/8000.0).sin()).collect::<Vec<_>>();
        *s=Snap::build(&mix,&[Some(Arc::new(Audio{sr:8000,l:v.clone(),r:v}))]);
    }
    fn engine(s: Snap) -> Engine { let mut e=Engine::new(8000.0,Arc::new(Shared::new()));e.handle(Cmd::Snap(Arc::new(s)));e.handle(Cmd::Play(true));e }
    fn output(e: &mut Engine,n:usize,chunk:usize)->Vec<f32> {let mut out=vec![0.0;n*2]; for c in out.chunks_mut(chunk*2){e.render(c,2);}out}
    #[test] fn maintenance_effect_identity_and_unchanged_snapshots() {
        let mut s=snap();tone(&mut s);let mut f=crate::fx::FxBlock::new(crate::fx::FxKind::Echo,4,0.0);f.time=0.25;s.fx.push(f.clone());
        let mut uninterrupted=engine(s);let mut s=snap();tone(&mut s);s.fx.push(f.clone());let mut updated=engine(s);
        let mut builder=SnapshotBuilder::default();builder.voices=updated.ws.len();builder.effects=updated.fx.iter().map(|f|f.0).collect();
        for _ in 0..32 {
            let mut s=snap();tone(&mut s);
            // Keep audio Arc identity, as editor snapshots do.
            s.tracks[0].audio=updated.snap.as_ref().unwrap().tracks[0].audio.clone();s.fx.push(f.clone());
            let p=builder.prepare(Arc::new(s),8000.0);assert!(p.ws.iter().all(Option::is_none));assert!(p.fx.iter().all(|f|f.1.is_none()));updated.handle(Cmd::Prepared(p));
            let aa=output(&mut uninterrupted,128,128);let bb=output(&mut updated,128,128);assert!(aa.iter().zip(bb).all(|(a,b)|(a-b).abs()<1e-6));
        }
        let mut silent=snap();silent.bpm=126.0;updated.handle(Cmd::Snap(Arc::new(silent)));output(&mut updated,2000,256);
        let mut silent=snap();silent.bpm=126.0;f.renew_id();silent.fx.push(f);updated.handle(Cmd::Snap(Arc::new(silent)));
        let out=output(&mut updated,12000,256);assert!(out.iter().all(|v|v.abs()<1e-6),"removed echo leaked into replacement");
    }
    #[test] fn maintenance_tempo_nodes_low_rates_and_tiny_loops() {
        for sr in [1.0f64,7.0,8000.0] {
            for range in [(0.0,0.0001),(0.125,2.75),(0.0,32.0)] {
                let mut s=snap();s.tempo=vec![(0.0,120.0),(1.0,240.0),(2.0,90.0),(20.0,180.0)];
                let expected=s.advance(range.0,(257.0/sr).rem_euclid(s.secs(range.0,range.1)));
                let mut e=Engine::new(sr as f32,Arc::new(Shared::new()));e.handle(Cmd::Snap(Arc::new(s)));e.handle(Cmd::Seek(range.0));e.handle(Cmd::Loop(Some(range)));e.handle(Cmd::Play(true));output(&mut e,257,31);
                assert!((e.beat-expected).abs()<1e-8,"rate {sr}, loop {range:?}: {} vs {expected}",e.beat);
            }
        }
    }
    #[test] fn maintenance_split_transition_audio_is_neutral() {
        for preset in [Preset::Crossfade,Preset::BassSwap] {
            let mut m=Mix::default();m.bpm=120.0;
            let mut a:crate::model::Track=serde_json::from_str(r#"{"path":"a","title":"tone","artist":"test","bpm":120.0,"offset":0.0,"key":"8A","duration":120.0,"start_beat":0.0,"lane":0,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap();a.stretch=2;
            let mut b=a.clone();b.path="b".into();b.lane=1;b.start_beat=160.0;b.trans.preset=preset;b.trans.curve_in=vec![(0.0,0.05),(0.5,0.7),(1.0,1.0)];m.tracks=vec![a,b];
            let audio=Arc::new(Audio{sr:8000,l:(0..960000).map(|i|0.15*(2.0*PI*330.0*i as f32/8000.0).sin()).collect(),r:vec![0.0;960000]});
            let audios=vec![Some(audio.clone()),Some(audio.clone())];let before=render_offline(Arc::new(Snap::build(&m,&audios)),8000,195.0,205.0).0;
            assert!(m.split(1,200.0).is_some());
            let after=render_offline(Arc::new(Snap::build(&m,&[Some(audio.clone()),Some(audio.clone()),Some(audio)])),8000,195.0,205.0).0;
            let diff=before.iter().zip(after).map(|(a,b)|(a-b).abs()).fold(0.0f32,f32::max);assert!(diff<1e-5,"{preset:?}: {diff}");
        }
    }
    #[test] fn maintenance_synthetic_callback_cost() {
        let mut s=snap();tone(&mut s);
        for _ in 0..16 {s.fx.push(crate::fx::FxBlock::new(crate::fx::FxKind::Echo,4,0.0));}
        let mut e=Engine::new(44100.0,Arc::new(Shared::new()));e.handle(Cmd::Snap(Arc::new(s)));e.handle(Cmd::Play(true));
        let start=std::time::Instant::now();let _=output(&mut e,44100*4,512);
        println!("synthetic callback: 4s stereo, 44.1kHz, WSOLA +16echo {:.3}s CPU",start.elapsed().as_secs_f64());
    }
    #[test] fn maintenance_ramp_loop_audio_is_callback_independent() {
        let make=|| {let mut s=snap();tone(&mut s);s.tempo=vec![(0.0,120.0),(1.0,240.0),(2.0,90.0)];let mut e=engine(s);e.handle(Cmd::Seek(0.125));e.handle(Cmd::Loop(Some((0.125,1.37))));e};
        let mut a=make();let mut b=make();let aa=output(&mut a,24000,256);let bb=output(&mut b,24000,31);
        assert!((a.beat-b.beat).abs()<1e-9);
        let diff=aa.iter().zip(bb).map(|(a,b)|(a-b).abs()).fold(0.0f32,f32::max);assert!(diff<1e-5,"{diff}");
    }
    #[test] fn maintenance_review_floor_tempo_inverse() {
        let duration=16.0+32.0*2.0f64.ln();
        for curve in [vec![(0.0,10.0),(16.0,40.0)],vec![(0.0,40.0),(16.0,10.0)]] {
            let mut s=snap();s.tempo=curve;
            assert!((s.advance(0.0,duration)-16.0).abs()<1e-10,"forward {}",s.advance(0.0,duration));
            assert!(s.advance(16.0,-duration).abs()<1e-10,"backward {}",s.advance(16.0,-duration));
        }
    }
    #[test] fn maintenance_review_late_fx_callback_independence() {
        for sr in [44100.0,48000.0] {for kind in crate::fx::FxKind::ALL {
            if kind==crate::fx::FxKind::Scratch {continue;}
            let make=|| {let mut s=snap();tone(&mut s);s.bpm=120.0;s.tracks[0].stretch=2;
                let mut f=crate::fx::FxBlock::new(kind,4,0.3333);f.mix=1.0;f.len=0.3333;f.tail=0.125;s.fx.push(f);
                let mut e=Engine::new(sr,Arc::new(Shared::new()));e.handle(Cmd::Snap(Arc::new(s)));e.handle(Cmd::Play(true));e};
            let a=output(&mut make(),sr as usize*2,512);let b=output(&mut make(),sr as usize*2,31);
            let diff=a.iter().zip(b).map(|(a,b)|(a-b).abs()).fold(0.0f32,f32::max);assert!(diff<1e-6,"{kind:?} {sr}: {diff}");
        }}
    }
    #[test] fn maintenance_review_one_sample_loop_reanchors_audio() {
        let mut s=snap();tone(&mut s);let step=s.bpm/60.0/8000.0;let mut e=engine(s);e.handle(Cmd::Seek(0.503));e.handle(Cmd::Loop(Some((0.503,0.503+step))));
        let mut dry=vec![];for _ in 0..512 {let mut out=[0.0;2];e.render(&mut out,2);dry.push(e.dbuf[0][0][0]);}
        let diff=dry.iter().map(|v|(v-dry[0]).abs()).fold(0.0f32,f32::max);assert!(diff<1e-6,"loop voice sample changed {diff}");
    }
    #[test] fn maintenance_stabilized_fragment_keylock_boundaries_are_continuous() {
        let sr=44100;let drift=crate::testgen::ramp_times(120.0,132.0,60.0,0.5);
        let (l,r)=crate::testgen::synth_kicks(&drift,61.5,sr);let audio=Arc::new(Audio{sr,l,r});
        let mut t:crate::model::Track=serde_json::from_str(r#"{"path":"synthetic","title":"drift","artist":"test","bpm":126.0,"offset":0.5,"key":"8A","duration":61.5,"start_beat":0.0,"lane":0,"stretch":1,"grid_mode":1,"trans":{"preset":"Crossfade","stems":["Full","Full","Full","Full"]}}"#).unwrap();
        t.map=Some(Arc::new(crate::beatmap::BeatMap{beats:drift.clone(),bpm:126.0}));
        let mut m=Mix::default();m.bpm=126.0;m.tempo_mode=1;m.decks[0].vol=1.0;m.tracks.push(t);
        let middle=m.split(0,32.5).unwrap();let last=m.split(middle,(drift.len()-1) as f64-32.5).unwrap();m.tracks[0].stabilize=true;m.tracks[last].stabilize=true;
        let from=m.tracks[0].vis_start(m.bpm);let to=m.tracks[last].end(m.bpm);
        let out=render_offline(Arc::new(Snap::build(&m,&[Some(audio.clone()),Some(audio.clone()),Some(audio)])),sr,from,to).0;
        for i in [middle,last] {
            let center=(m.secs(from,m.tracks[i].vis_start(m.bpm))*sr as f64) as usize;let width=(0.01*sr as f64) as usize;
            let jump=(center-width..center+width).map(|j|(out[j+1]-out[j]).abs()).fold(0.0f32,f32::max);
            assert!(jump<0.01,"fragment {i} boundary jump {jump}");
        }
    }
    #[test] fn maintenance_keylock_has_no_playlist_index_limit() {
        let mut a=snap();tone(&mut a);let expected=output(&mut engine(a),12000,256);
        let mut b=snap();tone(&mut b);
        for _ in 0..64 { let mut dead=snap();tone(&mut dead);let mut t=dead.tracks.remove(0);t.vis=-100.0;t.end=-90.0;b.tracks.insert(0,t); }
        let actual=output(&mut engine(b),12000,256);
        assert!(expected.iter().zip(&actual).all(|(a,b)|(a-b).abs()<1e-6));
    }
    #[test] fn maintenance_seventeenth_effect_is_audible() {
        let mut a=snap();tone(&mut a);let dry=output(&mut engine(a),16000,256);
        let mut b=snap();tone(&mut b);
        for _ in 0..16 { b.fx.push(crate::fx::FxBlock::new(crate::fx::FxKind::Scratch,0,0.0)); }
        let mut f=crate::fx::FxBlock::new(crate::fx::FxKind::FilterSweep,4,0.0);f.len=4.0;f.depth=1.0;b.fx.push(f);
        let wet=output(&mut engine(b),16000,256);
        let diff=wet.iter().zip(dry).map(|(a,b)|(a-b).abs()).fold(0.0f32,f32::max);assert!(diff>0.05,"{diff}");
    }
    #[test] fn maintenance_seek_resets_all_history() {
        let mut a=snap();tone(&mut a);a.decks[0].filter=-0.8;a.metro=Some(0.3);let mut e=engine(a);output(&mut e,10000,256);e.handle(Cmd::Seek(0.0));let actual=output(&mut e,4000,256);
        let mut b=snap();tone(&mut b);b.decks[0].filter=-0.8;b.metro=Some(0.3);let expected=output(&mut engine(b),4000,256);
        assert!(actual.iter().zip(expected).all(|(a,b)|(a-b).abs()<1e-6));
    }
    #[test] fn maintenance_loop_keeps_fractional_elapsed_time() {
        let mut s=snap();s.bpm=120.0;let mut e=engine(s);e.handle(Cmd::Loop(Some((0.0,0.1))));output(&mut e,4096,256);
        assert!((e.beat-0.024).abs()<1e-10,"{}",e.beat);
    }
    #[test] fn maintenance_ramp_is_callback_independent() {
        let mut a=snap();tone(&mut a);a.tempo=vec![(0.0,120.0),(16.0,240.0)];let duration=8.0*2.0f64.ln();let count=(duration*8000.0).round() as usize;
        let mut b=snap();tone(&mut b);b.tempo=a.tempo.clone();let mut ea=engine(a);let mut eb=engine(b);let aa=output(&mut ea,count,256);let bb=output(&mut eb,count,31);
        assert!((ea.beat-eb.beat).abs()<1e-9,"{} {}",ea.beat,eb.beat);
        assert!((ea.beat-16.0).abs()<0.001);let diff=aa.iter().zip(bb).map(|(a,b)|(a-b).abs()).fold(0.0f32,f32::max);assert!(diff<1e-5,"audio diff {diff}");
    }
}
