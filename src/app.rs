use crate::analysis::{self, Analysis, Audio, PEAK_HOP};
use crate::engine::{self, Cmd, Shared, Snap};
use crate::model::{Mix, Preset, StemMode, Track};
use eframe::egui::{self, pos2, vec2, Align2, Color32 as C, FontId, Rect, Rounding, Sense, Stroke};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const ACCENT: C = C::from_rgb(232, 224, 0);
const BG: C = C::from_rgb(26, 27, 31);
const PANEL: C = C::from_rgb(36, 38, 43);
const ROW: C = C::from_rgb(30, 32, 36);
const LINE: C = C::from_rgb(52, 55, 61);
const TXT: C = C::from_rgb(220, 222, 226);
const DIM: C = C::from_rgb(140, 144, 152);
const BLUE: C = C::from_rgb(84, 104, 236);
const OLIVE: C = C::from_rgb(98, 96, 22);
const STEM: [C; 4] = [C::from_rgb(58, 150, 92), C::from_rgb(212, 172, 74), C::from_rgb(206, 80, 72), C::from_rgb(76, 102, 200)];
const STEM_N: [&str; 4] = ["Drums", "Bass", "Melody", "Vocals"];
const HEAD_W: f32 = 132.0;
const RULER_H: f32 = 30.0;
const SAMPLE_H: f32 = 25.0;
const TEMPO_H: f32 = 22.0;

#[derive(PartialEq, Clone, Copy)]
enum Tab { Zoom, Library, Playlist, Transition, Track, Video, Effects, Master, Samples }
#[derive(PartialEq, Clone, Copy)]
enum Snapping { Off, Free, Half, Full }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BmMode { Grid, Following, Single }
impl BmMode {
    pub const ALL: [BmMode; 3] = [BmMode::Grid, BmMode::Following, BmMode::Single];
    pub fn name(self) -> &'static str { match self { BmMode::Grid => "⇔ Adjust full grid", BmMode::Following => "⇥ Beat + following", BmMode::Single => "• Single beat" } }
    pub fn help(self) -> &'static str { match self {
        BmMode::Grid => "Drag a beat marker to shift the whole beat grid",
        BmMode::Following => "Drag a beat marker: beats up to one bar to the left stay anchored, the following beats stretch (tempo change in %)",
        BmMode::Single => "Drag a beat marker to move only that beat" } }
}
fn bm_col(m: BmMode) -> C { match m { BmMode::Grid => C::from_rgb(90, 200, 255), BmMode::Following => C::from_rgb(255, 170, 40), BmMode::Single => C::from_rgb(120, 230, 120) } }
#[derive(Clone)]
struct BmDrag { ti: usize, k: i64, i: usize, pivot: usize, span: f64, lastx: f32, mode: BmMode }

enum Msg { Picked(Vec<PathBuf>, Option<usize>), Loaded(PathBuf, Result<(Audio, Analysis), String>, Option<(usize, f64)>), Status(String), Opened(PathBuf), SaveTo(PathBuf), ExportTo(PathBuf), Stems(String, Result<Arc<crate::stems::Stems>, String>), LibFolders(Vec<PathBuf>) }

struct StemJob { path: String, audio: Arc<Audio>, prog: Arc<std::sync::atomic::AtomicU32> }

pub struct App {
    mix: Mix, store: HashMap<String, (Arc<Audio>, Arc<Analysis>)>,
    out: crate::audio::Audio, ppb: f32, scroll: f64, tab: Tab, top: usize,
    sel: Option<usize>, sel_in: Option<usize>, undo: Vec<String>, redo: Vec<String>,
    tx: crossbeam_channel::Sender<Msg>, rx: crossbeam_channel::Receiver<Msg>, loading: usize,
    drag: Option<(usize, f64, f64)>, loop_on: bool, bars: u32, snapping: Snapping, lock: bool,
    file: Option<PathBuf>, status: String, auto_a: bool, follow: bool, grid_drag: Option<(usize, f32)>, bm_mode: BmMode, bm_drag: Option<BmDrag>, bm_pop: Option<(egui::Pos2, std::time::Instant)>, taps: Vec<std::time::Instant>,
    stems: HashMap<String, Arc<crate::stems::Stems>>, stem_prog: HashMap<String, Arc<std::sync::atomic::AtomicU32>>, stem_err: HashMap<String, String>,
    stem_tx: crossbeam_channel::Sender<StemJob>, dl_prog: Arc<std::sync::atomic::AtomicU32>, ep: Arc<Mutex<String>>,
    ao_mode: crate::automix::Mode, ao_curve: crate::automix::Curve, ao_keep_first: bool,
    lib: crate::library::Library, lib_q: String, lib_path: String, lib_sort: crate::library::SortCol, lib_asc: bool, lib_key: bool, lib_bpm: bool, lib_pct: f64,
    lib_sel: std::collections::HashSet<String>, lib_anchor: Option<usize>, lib_view: Vec<usize>,
    lib_view_key: Option<(u64, String, crate::library::SortCol, bool, Option<String>, Option<(u64, u64)>)>, timeline_rect: Rect,
}

pub fn run(demo: Option<String>) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("MixStudio").with_app_id("mixstudio")
            .with_inner_size([1440.0, 900.0]).with_min_inner_size([1000.0, 640.0]).with_icon(icon()).with_drag_and_drop(true),
        ..Default::default()
    };
    let _ = eframe::run_native("MixStudio", options, Box::new(move |cc| Ok(Box::new(App::new(cc, demo)))));
}

fn icon() -> egui::IconData { egui::IconData { rgba: icon_rgba(128), width: 128, height: 128 } }
pub fn icon_rgba(n: u32) -> Vec<u8> {
    let mut v = vec![0u8; (n * n * 4) as usize]; let c = n as f32 / 2.0;
    for y in 0..n { for x in 0..n {
        let (fx, fy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c); let i = ((y * n + x) * 4) as usize;
        if fx.abs().max(fy.abs()) / c > 0.94 { continue; }
        let mut col = [28u8, 30, 36];
        let u = (fx / c + 0.8) / 1.6 * 9.0; let bx = u.floor() as i32;
        if (0..9).contains(&bx) && u.fract() < 0.7 {
            let h = [0.25, 0.45, 0.7, 0.4, 0.85, 0.5, 0.65, 0.35, 0.2][bx as usize];
            if (fy / c).abs() < h * 0.75 { col = if bx < 4 { [70, 100, 236] } else { [232, 224, 0] }; }
        }
        v[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
    } }
    v
}
/// Minimal PNG encoder (stored deflate) for the icon file.
pub fn icon_png(n: u32) -> Vec<u8> {
    let rgba = icon_rgba(n); let w = n as usize * 4;
    let mut raw = Vec::new(); for y in 0..n as usize { raw.push(0u8); raw.extend(&rgba[y * w..(y + 1) * w]); }
    let mut z = vec![0x78, 0x01]; let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (k, ch) in chunks.iter().enumerate() { z.push(if k + 1 == chunks.len() { 1 } else { 0 }); let l = ch.len() as u16; z.extend(l.to_le_bytes()); z.extend((!l).to_le_bytes()); z.extend(*ch); }
    let (mut a, mut b) = (1u32, 0u32); for &x in &raw { a = (a + x as u32) % 65521; b = (b + a) % 65521; } z.extend(((b << 16) | a).to_be_bytes());
    fn crc(d: &[u8]) -> u32 { let mut c = 0xffffffffu32; for &x in d { c ^= x as u32; for _ in 0..8 { c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 }; } } !c }
    let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let mut chunk = |t: &[u8], d: &[u8]| { out.extend((d.len() as u32).to_be_bytes()); let mut td = t.to_vec(); td.extend(d); out.extend(&td); out.extend(crc(&td).to_be_bytes()); };
    let mut ih = vec![]; ih.extend(n.to_be_bytes()); ih.extend(n.to_be_bytes()); ih.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ih); chunk(b"IDAT", &z); chunk(b"IEND", &[]);
    out
}

fn fmt_t(s: f64) -> String { let s = s.max(0.0) as u64; if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) } }
fn key_col(k: &str) -> C {
    let n: f32 = k.trim_end_matches(['A', 'B']).parse().unwrap_or(1.0);
    let (r, g, b) = hsv((n - 1.0) / 12.0, if k.ends_with('A') { 0.35 } else { 0.5 }, 0.95); C::from_rgb(r, g, b)
}
fn hsv(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor(); let f = h * 6.0 - i; let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 { 0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t), 3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q) };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

impl App {
    fn new(cc: &eframe::CreationContext, demo: Option<String>) -> Self {
        let mut st = egui::Style::default(); st.visuals = egui::Visuals::dark();
        st.visuals.panel_fill = PANEL; st.visuals.window_fill = PANEL; st.visuals.extreme_bg_color = BG;
        st.visuals.selection.bg_fill = C::from_rgb(110, 106, 10); st.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        st.visuals.widgets.inactive.bg_fill = C::from_rgb(52, 54, 60); st.visuals.widgets.inactive.weak_bg_fill = C::from_rgb(52, 54, 60);
        st.visuals.widgets.hovered.weak_bg_fill = C::from_rgb(66, 68, 76);
        for w in [&mut st.visuals.widgets.inactive, &mut st.visuals.widgets.hovered, &mut st.visuals.widgets.active] { w.rounding = Rounding::same(6.0); }
        st.spacing.button_padding = vec2(8.0, 4.0);
        cc.egui_ctx.set_style(st);
        let (tx, rx) = crossbeam_channel::unbounded();
        // background stem-separation worker (one job at a time; the model session is reused)
        let (stem_tx, stem_rx) = crossbeam_channel::unbounded::<StemJob>();
        let dl_prog = Arc::new(std::sync::atomic::AtomicU32::new(0)); let ep = Arc::new(Mutex::new(String::new()));
        { let tx = tx.clone(); let dl = dl_prog.clone(); let ep = ep.clone(); let ctx = cc.egui_ctx.clone();
          std::thread::spawn(move || for j in stem_rx { let key = analysis::cache_key(std::path::Path::new(&j.path)).unwrap_or_else(|| "nokey".into());
            let r = crate::stems::get_or_separate(&key, &j.audio, &dl, &j.prog);
            if let Ok(e) = crate::stems::session_info() { *ep.lock().unwrap() = e; }
            let _ = tx.send(Msg::Stems(j.path, r)); ctx.request_repaint(); }); }
        let mut a = App { mix: Mix::default(), store: HashMap::new(), out: crate::audio::start(), ppb: 7.0, scroll: -4.0, tab: Tab::Playlist, top: 3,
            sel: None, sel_in: None, undo: vec![], redo: vec![], tx, rx, loading: 0, drag: None, loop_on: false, bars: 8, snapping: Snapping::Full, lock: false,
            file: None, status: String::new(), auto_a: true, follow: true, grid_drag: None, bm_mode: BmMode::Grid, bm_drag: None, bm_pop: None, taps: vec![], stems: HashMap::new(), stem_prog: HashMap::new(), stem_err: HashMap::new(), stem_tx, dl_prog, ep, ao_mode: Default::default(), ao_curve: Default::default(), ao_keep_first: true,
            lib: { let c = cc.egui_ctx.clone(); crate::library::Library::new(crate::library::config_dir(), crate::library::cache_dir(), Arc::new(move || c.request_repaint())) },
            lib_q: String::new(), lib_path: String::new(), lib_sort: crate::library::SortCol::Artist, lib_asc: true, lib_key: false, lib_bpm: false, lib_pct: 6.0,
            lib_sel: Default::default(), lib_anchor: None, lib_view: vec![], lib_view_key: None, timeline_rect: Rect::NOTHING };
        if !a.lib.cfg.folders.is_empty() { a.lib.rescan(); }
        if let Some(d) = demo {
            let paths = crate::gen_demo(std::path::Path::new(&d));
            for p in &paths { if let Ok((au, an)) = analysis::load(p) { a.insert(p.clone(), au, an, None); } }
            let pr = [Preset::Crossfade, Preset::BassSwap, Preset::EchoOut, Preset::HighToLow];
            for (i, t) in a.mix.tracks.iter_mut().enumerate() { t.trans.preset = pr[i % 4]; t.trans.stems = [StemMode::Swap, StemMode::Swap, StemMode::Full, StemMode::Full]; t.artist = ["Kick Lab", "Pad Unit", "Low End", "Raver"][i % 4].into(); }
            let s0 = a.mix.transitions().first().map(|x| x.2).unwrap_or(0.0);
            if let Some(p) = paths.get(1) { let (au, an) = analysis::load(p).unwrap(); a.insert(p.clone(), au, an, Some((2, s0 - 24.0))); }
            if let Some(p) = paths.get(1) { let (au, an) = analysis::load(p).unwrap(); a.insert(p.clone(), au, an, Some((3, s0 + 40.0))); }
            for t in a.mix.tracks.iter_mut().filter(|t| t.lane >= 2) { t.title = format!("{} vocals HQ", t.title); t.duration = t.duration.min(t.offset + 18.0); }
            a.mix.name = "Electric Vision 5".into();
            if let Some(&(_, i, s, e)) = a.mix.transitions().first() { a.sel_in = Some(i); a.scroll = s - 40.0; a.seek((s + e) / 2.0 - 4.0); }
            a.ppb = 7.5; a.tab = Tab::Track; a.sel = Some(1); a.mix.metronome = true; a.follow = false;
            match std::env::var("MIXSTUDIO_DEMO_TAB").as_deref() { Ok("transition") => { a.tab = Tab::Transition; a.sel = None; }
                Ok("playlist") => { a.tab = Tab::Playlist; a.sel = Some(1); } Ok("wide") => { a.tab = Tab::Playlist; a.ppb = 3.0; a.scroll = -4.0; a.sel = None; } _ => {} }
        }
        a
    }
    fn snapshot(&mut self) { self.undo.push(serde_json::to_string(&self.mix).unwrap()); if self.undo.len() > 200 { self.undo.remove(0); } self.redo.clear(); }
    fn do_undo(&mut self) { if let Some(s) = self.undo.pop() { self.redo.push(serde_json::to_string(&self.mix).unwrap()); self.mix = serde_json::from_str(&s).unwrap(); self.fix_sel(); } }
    fn do_redo(&mut self) { if let Some(s) = self.redo.pop() { self.undo.push(serde_json::to_string(&self.mix).unwrap()); self.mix = serde_json::from_str(&s).unwrap(); self.fix_sel(); } }
    fn fix_sel(&mut self) { let n = self.mix.tracks.len(); if self.sel.is_some_and(|s| s >= n) { self.sel = None; } if self.sel_in.is_some_and(|s| s >= n) { self.sel_in = None; } }
    fn beat(&self) -> f64 { self.out.shared.beat() }
    fn playing(&self) -> bool { self.out.shared.playing.load(std::sync::atomic::Ordering::Relaxed) }
    fn seek(&self, b: f64) { self.out.shared.beat.store(b.max(0.0).to_bits(), std::sync::atomic::Ordering::Relaxed); let _ = self.out.tx.send(Cmd::Seek(b.max(0.0))); }
    fn sec(&self, b: f64) -> f64 { b * 60.0 / self.mix.bpm }

    fn insert(&mut self, p: PathBuf, au: Audio, an: Analysis, sample: Option<(usize, f64)>) {
        let key = p.display().to_string();
        let t = Track { path: key.clone(), title: an.title.clone(), artist: if an.artist.is_empty() { "Unknown artist".into() } else { an.artist.clone() }, bpm: an.bpm, offset: an.offset,
            key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false };
        self.store.insert(key, (Arc::new(au), Arc::new(an)));
        match sample { Some((lane, at)) => { let mut t = t; t.lane = lane; t.start_beat = (at / 4.0).round() * 4.0; self.mix.tracks.push(t); }
            None => self.mix.add_track(t) }
        // load cached stems right away, or separate automatically if enabled
        let last = self.mix.tracks.len() - 1;
        let cached = analysis::cache_key(&p).map(|k| crate::stems::cache_file(&k).exists()).unwrap_or(false);
        if cached || self.mix.auto_stems { self.request_stems(last); }
    }
    fn pick_files(&self, sample: Option<usize>) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let f = ("Audio".to_string(), crate::library::EXTS.iter().map(|s| s.to_string()).collect());
            match crate::dialogs::run(crate::dialogs::Req::Files { title: "Add tracks".into(), filter: f }) {
                Ok(Some(v)) => { let _ = tx.send(Msg::Picked(v, sample)); } Ok(None) => {}
                Err(e) => { let _ = tx.send(Msg::Status(format!("Add tracks: {e}"))); } }
        });
    }
    fn load_files(&mut self, v: Vec<PathBuf>, sample: Option<usize>) {
        self.snapshot(); self.loading += v.len();
        let tx = self.tx.clone(); let at = self.beat();
        std::thread::spawn(move || for p in v {
            let r = analysis::load(&p).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Loaded(p, r, sample.map(|l| (l, at))));
        });
    }
    fn save(&mut self, p: PathBuf) {
        match std::fs::write(&p, serde_json::to_string_pretty(&self.mix).unwrap()) { Ok(_) => { self.status = format!("Saved: {}", p.display()); self.file = Some(p); } Err(e) => self.status = format!("Could not save: {e}") }
    }
    fn open(&mut self, p: PathBuf) {
        match std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Mix>(&s).ok()) {
            Some(m) => {
                self.snapshot(); let paths: Vec<String> = m.tracks.iter().map(|t| t.path.clone()).filter(|x| !self.store.contains_key(x)).collect();
                self.mix = m; self.file = Some(p); self.fix_sel();
                self.loading += paths.len(); let tx = self.tx.clone();
                std::thread::spawn(move || for p in paths { let pb = PathBuf::from(&p);
                    let r = analysis::load(&pb).map_err(|e| e.to_string()); let _ = tx.send(Msg::Loaded(pb, r, Some((usize::MAX, 0.0)))); });
            }
            None => self.status = "Could not read the mix file".into(),
        }
    }
    fn dialog(&self, save: bool, kind: u8) {
        let tx = self.tx.clone(); let name = self.mix.name.clone();
        std::thread::spawn(move || {
            use crate::dialogs::Req;
            let filter = if kind == 2 { ("WAV".to_string(), vec!["wav".to_string()]) } else { ("MixStudio mix".to_string(), vec!["json".to_string()]) };
            let title = match kind { 0 => "Open mix", 1 => "Save mix", _ => "Export WAV" }.to_string();
            let req = if save { Req::Save { title, filter, name: if kind == 2 { format!("{name}.wav") } else { format!("{name}.mixstudio.json") } } } else { Req::Open { title, filter } };
            let r = match crate::dialogs::run(req) { Ok(v) => v.and_then(|v| v.into_iter().next()), Err(e) => { let _ = tx.send(Msg::Status(e)); None } };
            if let Some(p) = r { let _ = tx.send(match kind { 0 => Msg::Opened(p), 1 => Msg::SaveTo(p), _ => Msg::ExportTo(p) }); }
        });
    }
    fn build_snap(&self) -> Snap {
        let auds: Vec<Option<Arc<Audio>>> = self.mix.tracks.iter().map(|t| self.store.get(&t.path).map(|x| x.0.clone())).collect();
        let st: Vec<Option<Arc<crate::stems::Stems>>> = self.mix.tracks.iter().map(|t| self.stems.get(&t.path).cloned()).collect();
        Snap::build_with(&self.mix, &auds, &st)
    }
    fn export(&mut self, p: PathBuf) {
        let snap = Arc::new(self.build_snap()); let len = self.mix.length_beats() + 16.0; let tx = self.tx.clone();
        self.status = "Exporting mix…".into();
        std::thread::spawn(move || { let (l, r) = engine::render_offline(snap, 44100, 0.0, len);
            let _ = tx.send(Msg::Status(match engine::write_wav(&p, &l, &r, 44100) { Ok(_) => format!("Exported: {}", p.display()), Err(e) => format!("Export error: {e}") })); });
    }
    fn ao_item(&self, i: usize) -> crate::automix::Item {
        let t = &self.mix.tracks[i];
        let energy = self.store.get(&t.path).map(|(_, an)| an.peaks.iter().map(|p| (p[0] as f64 + p[1] as f64) / 510.0).sum::<f64>() / an.peaks.len().max(1) as f64).unwrap_or(0.5);
        crate::automix::Item { key: t.key.clone(), bpm: t.bpm, energy }
    }
    /// Reorder the main-deck tracks for key and tempo flow (and the chosen energy curve); undoable.
    fn auto_order(&mut self) {
        let o = self.mix.main_order(); if o.len() < 3 { self.status = "Auto-order needs at least 3 tracks".into(); return; }
        let items: Vec<_> = o.iter().map(|&i| self.ao_item(i)).collect();
        let locked: Vec<bool> = o.iter().enumerate().map(|(k, &i)| self.mix.tracks[i].locked || (k == 0 && self.ao_keep_first)).collect();
        let opts = crate::automix::Opts { mode: self.ao_mode, curve: self.ao_curve };
        let t0 = std::time::Instant::now();
        let ord = crate::automix::optimize(&items, &locked, &opts);
        let (c0, c1) = (crate::automix::order_cost(&items, &(0..o.len()).collect::<Vec<_>>(), &opts), crate::automix::order_cost(&items, &ord, &opts));
        let new: Vec<usize> = ord.iter().map(|&k| o[k]).collect();
        if new == o { self.status = "Auto-order: the current order is already the best one found".into(); return; }
        self.snapshot();
        let bars = self.bars as f64; self.mix.relayout(&new, bars);
        let greens = ord.windows(2).filter(|w| crate::automix::rate(&items[w[0]], &items[w[1]]).1 == 0).count();
        self.status = format!("Auto-order ({}): {greens}/{} smooth transitions, score {:.2} → {:.2} in {} ms. Undo restores the old order.", self.ao_mode.name(), ord.len() - 1, c0, c1, t0.elapsed().as_millis());
    }
    fn remove_sel(&mut self) {
        if let Some(i) = self.sel { self.snapshot(); self.mix.tracks.remove(i); self.sel = None; self.sel_in = None;
            let o = self.mix.main_order(); for (k, &i) in o.iter().enumerate() { self.mix.tracks[i].lane = k % 2; } }
    }
    /// Make the audio at mix beat `beat` (inside track ti) the downbeat, placed on the nearest mix bar.
    fn set_downbeat_at(&mut self, ti: usize, beat: f64) {
        let t = &mut self.mix.tracks[ti];
        let sec = t.sec_at(beat); let target = (beat / 4.0).round() * 4.0;
        t.offset = sec; t.start_beat = target; t.normalize_grid();
    }
    /// Shift the beat grid (fixed or beat map) relative to the audio by `dsec` seconds.
    fn nudge(&mut self, ti: usize, dsec: f64) {
        let t = &mut self.mix.tracks[ti];
        t.offset += dsec; t.normalize_grid();
    }
    fn request_stems(&mut self, i: usize) {
        let Some(t) = self.mix.tracks.get(i) else { return }; let p = t.path.clone();
        if self.stems.contains_key(&p) || self.stem_prog.contains_key(&p) { return; }
        let Some((au, _)) = self.store.get(&p) else { return };
        let prog = Arc::new(std::sync::atomic::AtomicU32::new(0)); self.stem_prog.insert(p.clone(), prog.clone()); self.stem_err.remove(&p);
        let _ = self.stem_tx.send(StemJob { path: p, audio: au.clone(), prog });
    }
    fn snap_beat(&self, b: f64) -> f64 { match self.snapping { Snapping::Off => b, Snapping::Free => b.round(), Snapping::Half => (b / 2.0).round() * 2.0, Snapping::Full => (b / 4.0).round() * 4.0 } }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        while let Ok(m) = self.rx.try_recv() { match m {
            Msg::Picked(v, s) => self.load_files(v, s),
            Msg::Loaded(p, r, s) => { self.loading = self.loading.saturating_sub(1); match r {
                Ok((au, an)) => { if s.is_some_and(|x| x.0 == usize::MAX) { let k = p.display().to_string(); self.store.insert(k.clone(), (Arc::new(au), Arc::new(an)));
                        let cached = analysis::cache_key(&p).map(|c| crate::stems::cache_file(&c).exists()).unwrap_or(false);
                        if cached || self.mix.auto_stems { if let Some(i) = self.mix.tracks.iter().position(|t| t.path == k) { self.request_stems(i); } } }
                    else { self.status = format!("{} — {:.1} BPM, {}", an.title, an.bpm, an.key); self.insert(p, au, an, s); } }
                Err(e) => self.status = format!("Could not read {}: {e}", p.display()) } }
            Msg::Stems(p, r) => { self.stem_prog.remove(&p); match r { Ok(st) => { self.stems.insert(p.clone(), st); self.stem_err.remove(&p); self.status = "Stems ready".into(); }
                Err(e) => { self.status = format!("Stem separation failed: {e}"); self.stem_err.insert(p, e); } } }
            Msg::LibFolders(v) => { self.add_lib_folders(v); }
            Msg::Status(s) => self.status = s, Msg::Opened(p) => self.open(p), Msg::SaveTo(p) => self.save(p), Msg::ExportTo(p) => self.export(p),
        } }
        self.lib.poll();
        let dropped: Vec<egui::DroppedFile> = ctx.input(|i| i.raw.dropped_files.clone());
        let mut files: Vec<PathBuf> = vec![];
        for f in dropped {
            if let Some(p) = f.path { files.push(p); }
            else if let Some(bytes) = f.bytes {
                let dir = std::env::temp_dir().join("mixstudio-drops"); let _ = std::fs::create_dir_all(&dir);
                let name = if f.name.is_empty() { format!("drop-{}.wav", files.len()) } else { f.name.clone() };
                let p = dir.join(name); if std::fs::write(&p, &bytes[..]).is_ok() { files.push(p); }
            }
        }
        let dirs: Vec<PathBuf> = files.iter().filter(|p| p.is_dir()).cloned().collect();
        if !dirs.is_empty() { self.add_lib_folders(dirs); }
        files.retain(|p| crate::library::is_audio(p));
        if !files.is_empty() { self.load_files(files, None); }
        let _ = self.out.tx.send(Cmd::Snap(Arc::new(self.build_snap())));
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) { let _ = self.out.tx.send(Cmd::Play(!self.playing())); }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) { if ctx.input(|i| i.modifiers.shift) { self.do_redo() } else { self.do_undo() } }
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) { self.remove_sel(); }
        }
        if self.playing() || self.loading > 0 || self.lib.scanning { ctx.request_repaint(); } else { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
        self.top_bar(ctx);
        self.tab_bar(ctx);
        egui::TopBottomPanel::bottom("bottom").exact_height(if self.tab == Tab::Library { 340.0 } else { 232.0 }).frame(egui::Frame::none().fill(C::from_rgb(28, 29, 33)).inner_margin(10.0)).show(ctx, |ui| self.bottom(ui));
        self.transport(ctx);
        egui::SidePanel::right("master").exact_width(42.0).resizable(false).frame(egui::Frame::none().fill(BG)).show(ctx, |ui| self.master_meter(ui));
        self.timeline_rect = egui::CentralPanel::default().frame(egui::Frame::none().fill(BG)).show(ctx, |ui| self.timeline(ui)).response.rect;
        // drag from the library onto the timeline (or the tab bar / playlist area) adds the tracks to the mix
        if let Some(p) = egui::DragAndDrop::payload::<LibDrag>(ctx) {
            let pos = ctx.pointer_latest_pos();
            egui::show_tooltip_at_pointer(ctx, egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("libdrag")), egui::Id::new("libdrag_tip"), |ui| { ui.label(format!("Add {} track(s) to the mix", p.0.len())); });
            if ctx.input(|i| i.pointer.any_released()) { egui::DragAndDrop::clear_payload(ctx);
                if pos.is_some_and(|q| self.timeline_rect.contains(q)) { self.lib_add(p.0.clone()); } }
        }
        let hovering = ctx.input(|i| i.raw.hovered_files.len());
        if hovering > 0 {
            let r = ctx.screen_rect(); let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            p.rect_filled(r, 0.0, C::from_rgba_unmultiplied(20, 20, 0, 160));
            p.rect_stroke(r.shrink(8.0), 12.0, Stroke::new(3.0_f32, ACCENT));
            p.text(r.center(), Align2::CENTER_CENTER, format!("Drop {} file(s) to add to the mix", hovering), FontId::proportional(24.0), ACCENT);
        }
    }
}

fn seg_button(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).color(if on { ACCENT } else { TXT }))
        .fill(if on { C::from_rgb(60, 58, 20) } else { C::TRANSPARENT }).stroke(if on { Stroke::new(1.0_f32, ACCENT) } else { Stroke::NONE }))
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").exact_height(44.0).frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(10.0, 6.0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if ui.add(egui::Button::new("🏠").min_size(vec2(30.0, 28.0))).on_hover_text("New empty mix").clicked() && !self.mix.tracks.is_empty() { self.snapshot(); self.mix = Mix::default(); self.sel = None; self.sel_in = None; }
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new("Mix").color(ACCENT).size(20.0).strong());
                ui.label(egui::RichText::new("Studio").color(TXT).size(20.0).strong());
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(14.0);
                let name = format!("☰  {}   ⏷", self.mix.name);
                ui.scope(|ui| {
                    let wv = &mut ui.visuals_mut().widgets; for w in [&mut wv.inactive, &mut wv.hovered, &mut wv.open] { w.bg_stroke = Stroke::new(1.5_f32, ACCENT); }
                    ui.menu_button(egui::RichText::new(name).size(14.0), |ui| {
                        ui.horizontal(|ui| { ui.label("Name:"); ui.text_edit_singleline(&mut self.mix.name); });
                        ui.separator();
                        if ui.button("New mix").clicked() { self.snapshot(); self.mix = Mix::default(); self.sel = None; self.file = None; ui.close_menu(); }
                        if ui.button("Open mix…").clicked() { self.dialog(false, 0); ui.close_menu(); }
                        if ui.button("Save").clicked() { match self.file.clone() { Some(p) => self.save(p), None => self.dialog(true, 1) } ui.close_menu(); }
                        if ui.button("Save as…").clicked() { self.dialog(true, 1); ui.close_menu(); }
                    });
                });
                let mid = ui.max_rect().center().x; let cur = ui.cursor().min.x; ui.add_space((mid - 200.0 - cur).max(10.0));
                egui::Frame::none().fill(C::from_rgb(28, 29, 33)).rounding(8.0).inner_margin(3.0).show(ui, |ui| { ui.horizontal(|ui| {
                    for (k, n) in ["▭ Carousel", "☰ Playlist", "|||\\ Library", "〰 Studio"].iter().enumerate() {
                        if seg_button(ui, n, self.top == k).on_hover_text(if k == 0 { "Coming in v2" } else { "" }).clicked() { self.top = k; if k == 1 { self.tab = Tab::Playlist; } if k == 2 { self.tab = Tab::Library; } }
                    } }); });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⚙", |ui| {
                        ui.label(egui::RichText::new("Settings").strong());
                        ui.checkbox(&mut self.mix.auto_stems, "Separate stems automatically for new tracks");
                        if ui.button("Separate stems for all tracks").clicked() { for i in 0..self.mix.tracks.len() { self.request_stems(i); } ui.close_menu(); }
                        ui.separator();
                        let ep = self.ep.lock().unwrap().clone();
                        ui.label(egui::RichText::new(format!("Stem model: HT-Demucs (MIT), {} MB, downloaded on first use to ~/.local/share/mixstudio/models", crate::stems::MODEL_SIZE / 1_000_000)).color(DIM).size(11.0));
                        ui.label(egui::RichText::new(if ep.is_empty() { "Inference: not started yet (CUDA if available, else CPU)".to_string() } else { format!("Inference: {ep}") }).color(DIM).size(11.0));
                    });
                    ui.add(egui::Button::new("?").min_size(vec2(28.0, 28.0))).on_hover_text("Space = play/pause · Ctrl+scroll = zoom · scroll = pan · Ctrl+Z = undo · Delete = remove track · drop audio files on the window");
                    if ui.add(egui::Button::new("Export").min_size(vec2(60.0, 28.0))).on_hover_text("Export the whole mix to WAV").clicked() { self.dialog(true, 2); }
                    if self.loading > 0 { ui.spinner(); ui.label(format!("Analysing {}…", self.loading)); }
                    else if !self.status.is_empty() { ui.label(egui::RichText::new(&self.status).color(DIM).size(12.0)); }
                });
            });
        });
    }

    fn tab_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("tabs").exact_height(40.0).frame(egui::Frame::none().fill(PANEL).inner_margin(6.0)).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let tabs = [(Tab::Zoom, "🔍 Zoom"), (Tab::Library, "📚 Library"), (Tab::Playlist, "☰ Playlist"), (Tab::Transition, "🔀 Transition"), (Tab::Track, "♫ Track"), (Tab::Video, "🎥 Video"), (Tab::Effects, "✨ Effects"), (Tab::Master, "🔊 Master"), (Tab::Samples, "⚙ Samples")];
                ui.add_space((ui.available_width() / 2.0 - 320.0).max(0.0));
                egui::Frame::none().fill(C::from_rgb(28, 29, 33)).rounding(8.0).inner_margin(3.0).show(ui, |ui| { ui.horizontal(|ui| {
                    for (t, n) in tabs { if seg_button(ui, n, self.tab == t).clicked() { self.tab = t; } } }); });
            });
        });
    }

    fn transport(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("transport").exact_height(46.0).frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(10.0, 6.0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let circ = |ui: &mut egui::Ui, t: &str, fill: C, fg: C, tip: &str| ui.add(egui::Button::new(egui::RichText::new(t).color(fg).size(16.0)).fill(fill).rounding(16.0).min_size(vec2(32.0, 32.0)).stroke(Stroke::new(1.0_f32, if fg == ACCENT { ACCENT } else { LINE }))).on_hover_text(tip);
                if circ(ui, "+", PANEL, ACCENT, "Add tracks (mp3, wav, flac, ogg, m4a)").clicked() { self.pick_files(None); }
                if ui.add(egui::Button::new(egui::RichText::new("Auto-order").color(C::BLACK).size(14.0)).fill(ACCENT).rounding(16.0).min_size(vec2(96.0, 30.0))).on_hover_text("Reorder the playlist for smooth key (Camelot) and tempo transitions. Locked tracks stay put. Undo restores the old order.").clicked() { self.auto_order(); }
                ui.menu_button("⏷", |ui| { ui.label(egui::RichText::new("Auto-order").strong());
                    for m in crate::automix::Mode::ALL { ui.radio_value(&mut self.ao_mode, m, m.name()); }
                    ui.separator(); ui.label(egui::RichText::new("Energy").color(DIM));
                    for c in crate::automix::Curve::ALL { ui.radio_value(&mut self.ao_curve, c, c.name()); }
                    ui.separator(); ui.checkbox(&mut self.ao_keep_first, "Keep first track in place");
                    ui.label(egui::RichText::new("Lock tracks with 🔒 in the playlist.").color(DIM).size(11.0));
                    if ui.button("Auto-order now").clicked() { self.auto_order(); ui.close_menu(); } }).response.on_hover_text("Auto-order options");
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("⟲").rounding(16.0).min_size(vec2(30.0, 30.0))).on_hover_text("Undo (Ctrl+Z)").clicked() { self.do_undo(); }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("⟳").rounding(16.0).min_size(vec2(30.0, 30.0))).on_hover_text("Redo (Ctrl+Shift+Z)").clicked() { self.do_redo(); }
                if ui.add(egui::Button::new("✂").rounding(16.0).min_size(vec2(30.0, 30.0))).on_hover_text("Remove selected track").clicked() { self.remove_sel(); }
                ui.menu_button("Edit  ⏷", |ui| {
                    if ui.button("Add tracks…").clicked() { self.pick_files(None); ui.close_menu(); }
                    if ui.button("Remove selected track").clicked() { self.remove_sel(); ui.close_menu(); }
                    if ui.button(format!("Set all overlaps to {} bars", self.bars)).clicked() { self.snapshot(); let o = self.mix.main_order(); self.mix.relayout(&o, self.bars as f64); ui.close_menu(); }
                    if ui.button("Open mix…").clicked() { self.dialog(false, 0); ui.close_menu(); }
                    if ui.button("Save mix as…").clicked() { self.dialog(true, 1); ui.close_menu(); }
                    if ui.button("Export WAV…").clicked() { self.dialog(true, 2); ui.close_menu(); }
                });
                egui::ComboBox::from_id_salt("bars").width(46.0).selected_text(format!("{}", self.bars))
                    .show_ui(ui, |ui| { for b in [4, 8, 16, 32] { ui.selectable_value(&mut self.bars, b, format!("{b} bars")); } }).response.on_hover_text("Bar length for loop and overlap");
                let mid = ui.max_rect().center().x; let cur = ui.cursor().min.x; ui.add_space((mid - 125.0 - cur).max(6.0));
                let flat = |ui: &mut egui::Ui, t: &str, tip: &str| ui.add(egui::Button::new(egui::RichText::new(t).size(17.0)).frame(false).min_size(vec2(30.0, 30.0))).on_hover_text(tip);
                let b = self.beat(); let bpm = self.mix.bpm;
                if flat(ui, "⏮", "Previous track").clicked() { let st: Vec<f64> = self.mix.main_order().iter().map(|&i| self.mix.tracks[i].vis_start(bpm)).filter(|&s| s < b - 2.0).collect(); self.seek(st.last().copied().unwrap_or(0.0)); }
                if flat(ui, "⏪", "To start").clicked() { self.seek(0.0); }
                if circ(ui, "⏺", C::from_rgb(50, 52, 58), C::from_rgb(230, 40, 40), "Record: export the mix to WAV").clicked() { self.dialog(true, 2); }
                let pl = self.playing();
                if ui.add(egui::Button::new(egui::RichText::new(if pl { "⏸" } else { "▶" }).color(C::BLACK).size(18.0)).fill(ACCENT).rounding(18.0).min_size(vec2(38.0, 36.0))).on_hover_text("Play/pause (space)").clicked() { let _ = self.out.tx.send(Cmd::Play(!pl)); }
                let lb = egui::Button::new(egui::RichText::new("🔁").size(16.0).color(if self.loop_on { ACCENT } else { TXT })).frame(false).min_size(vec2(30.0, 30.0));
                if ui.add(lb).on_hover_text("Loop the selected bar length from the current bar").clicked() { self.loop_on = !self.loop_on;
                    let s = (b / 4.0).floor() * 4.0; let _ = self.out.tx.send(Cmd::Loop(if self.loop_on { Some((s, s + self.bars as f64 * 4.0)) } else { None })); }
                if flat(ui, "⏭", "Next track").clicked() { if let Some(s) = self.mix.main_order().iter().map(|&i| self.mix.tracks[i].vis_start(bpm)).find(|&s| s > b + 0.5) { self.seek(s); } }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Button::new("⌨").min_size(vec2(30.0, 28.0))).on_hover_text("Space: play · Ctrl+scroll: zoom · Scroll: pan · Delete: remove · Ctrl+Z / Ctrl+Shift+Z");
                    let mo = self.mix.metronome;
                    if ui.add(egui::Button::new(egui::RichText::new("A").strong().color(if mo { C::BLACK } else { TXT })).fill(if mo { ACCENT } else { C::from_rgb(52, 54, 60) }).min_size(vec2(30.0, 28.0))).on_hover_text("Metronome: click on mix beats, accented on the downbeat (never exported). Volume in the Master tab.").clicked() { self.mix.metronome = !mo; }
                    ui.add(egui::Button::new("🎧").min_size(vec2(30.0, 28.0))).on_hover_text("Headphone cue (v2)");
                    if ui.add(egui::Button::new(if self.lock { "🔒" } else { "🔓" }).min_size(vec2(30.0, 28.0))).on_hover_text("Lock tracks so they can't be dragged").clicked() { self.lock = !self.lock; }
                    let n = match self.snapping { Snapping::Off => "Off", Snapping::Free => "Free", Snapping::Half => "Half", Snapping::Full => "Full" };
                    egui::ComboBox::from_id_salt("snap").width(60.0).selected_text(n).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.snapping, Snapping::Off, "Off (no snap)"); ui.selectable_value(&mut self.snapping, Snapping::Free, "Free (beat)");
                        ui.selectable_value(&mut self.snapping, Snapping::Half, "Half (half bar)"); ui.selectable_value(&mut self.snapping, Snapping::Full, "Full (bar)"); }).response.on_hover_text("Snap when dragging tracks");
                });
            });
        });
    }

    fn master_meter(&mut self, ui: &mut egui::Ui) {
        let r = ui.max_rect();
        let m = [Shared::get(&self.out.shared.meter[0]), Shared::get(&self.out.shared.meter[1])];
        let c = pos2(r.center().x, r.bottom() - 20.0);
        let resp = ui.interact(Rect::from_center_size(c, vec2(26.0, 26.0)), ui.id().with("auto"), Sense::click()).on_hover_text("Auto-gain / master limiter");
        if resp.clicked() { self.auto_a = !self.auto_a; }
        let p = ui.painter();
        p.text(pos2(r.center().x, r.top() + 14.0), Align2::CENTER_CENTER, format!("{:.0}%", m[0].max(m[1]) * 100.0), FontId::proportional(13.0), TXT);
        let top = r.top() + 30.0; let bot = r.bottom() - 40.0;
        for ch in 0..2 {
            let x = r.left() + 9.0 + ch as f32 * 14.0;
            p.rect_filled(Rect::from_min_max(pos2(x, top), pos2(x + 9.0, bot)), 1.0, C::from_rgb(20, 20, 22));
            let f = ((20.0 * m[ch].max(1e-5).log10() + 48.0) / 48.0).clamp(0.0, 1.0);
            let n = 48; for k in 0..n { let fr = k as f32 / n as f32; if fr > f { break; }
                let y1 = bot - fr * (bot - top); let y0 = bot - (k + 1) as f32 / n as f32 * (bot - top) + 1.0;
                let col = if fr > 0.88 { C::from_rgb(220, 50, 40) } else if fr > 0.72 { C::from_rgb(220, 190, 40) } else { C::from_rgb(60, 200, 70) };
                p.rect_filled(Rect::from_min_max(pos2(x, y0), pos2(x + 9.0, y1)), 0.0, col); }
        }
        let col = if self.auto_a { ACCENT } else { DIM };
        p.circle_stroke(c, 11.0, Stroke::new(2.0_f32, col)); p.text(c, Align2::CENTER_CENTER, "A", FontId::proportional(14.0), col);
    }

    fn knob(ui: &mut egui::Ui, id: egui::Id, c: egui::Pos2, v: &mut f32, label: &str) {
        let resp = ui.interact(Rect::from_center_size(c, vec2(22.0, 22.0)), id, Sense::click_and_drag());
        if resp.dragged() { *v = (*v - resp.drag_delta().y * 0.01).clamp(-1.0, 1.0); }
        if resp.double_clicked() { *v = 0.0; }
        let p = ui.painter();
        p.circle_filled(c, 11.0, C::from_rgb(58, 60, 66)); p.circle_stroke(c, 11.0, Stroke::new(1.0_f32, C::from_rgb(90, 92, 98)));
        if v.abs() > 0.005 { let a0 = -std::f32::consts::FRAC_PI_2; let a1 = a0 + *v * 2.4;
            let pts: Vec<egui::Pos2> = (0..=12).map(|k| { let a = a0 + (a1 - a0) * k as f32 / 12.0; c + vec2(a.cos(), a.sin()) * 11.0 }).collect();
            p.add(egui::Shape::line(pts, Stroke::new(2.0_f32, ACCENT))); }
        let txt = if label == "Filter" { format!("{:.0}", *v * 100.0) } else { format!("{:.1}", *v * 12.0) };
        p.text(c, Align2::CENTER_CENTER, txt, FontId::proportional(9.5), TXT);
        p.text(c + vec2(0.0, 18.0), Align2::CENTER_CENTER, label, FontId::proportional(10.0), DIM);
        resp.on_hover_text(format!("{label}: drag up/down, double-click = reset"));
    }

    fn small_btn(ui: &mut egui::Ui, id: egui::Id, r: Rect, t: &str, on: bool, oncol: C) -> bool {
        let resp = ui.interact(r, id, Sense::click()); let p = ui.painter();
        p.rect_filled(r, 2.0, if on { oncol } else if resp.hovered() { C::from_rgb(70, 72, 78) } else { C::from_rgb(54, 56, 62) });
        p.text(r.center(), Align2::CENTER_CENTER, t, FontId::proportional(10.0), if on { C::BLACK } else { TXT });
        resp.clicked()
    }

    fn fader(ui: &mut egui::Ui, id: egui::Id, r: Rect, v: &mut f32, meter: f32) {
        let resp = ui.interact(r, id, Sense::click_and_drag());
        if resp.dragged() || resp.clicked() { if let Some(p) = resp.interact_pointer_pos() { *v = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0); } }
        if resp.double_clicked() { *v = 0.85; }
        let p = ui.painter();
        let mr = Rect::from_min_max(pos2(r.left(), r.center().y - 4.0), pos2(r.right(), r.center().y - 1.0));
        p.rect_filled(mr, 0.0, C::from_rgb(40, 42, 46));
        let f = ((20.0 * meter.max(1e-5).log10() + 42.0) / 42.0).clamp(0.0, 1.0);
        let segs = 16; for k in 0..segs { let fr = k as f32 / segs as f32; if fr >= f { break; }
            let x0 = mr.left() + fr * mr.width(); let col = if fr > 0.85 { C::from_rgb(220, 50, 40) } else if fr > 0.7 { C::from_rgb(220, 190, 40) } else { C::from_rgb(60, 200, 70) };
            p.rect_filled(Rect::from_min_max(pos2(x0, mr.top()), pos2(x0 + mr.width() / segs as f32 - 1.0, mr.bottom())), 0.0, col); }
        p.rect_filled(Rect::from_min_max(pos2(r.left(), r.center().y + 1.0), pos2(r.right(), r.center().y + 3.0)), 1.0, C::from_rgb(70, 72, 78));
        p.rect_filled(Rect::from_center_size(pos2(r.left() + *v * r.width(), r.center().y), vec2(6.0, 13.0)), 1.5, C::from_rgb(210, 212, 216));
        resp.on_hover_text(format!("Volume {:.0}%", *v * 100.0));
    }
}

impl App {
    fn timeline(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        let mini_h = 14.0;
        let mini = Rect::from_min_max(full.min, pos2(full.right(), full.top() + mini_h));
        let area = Rect::from_min_max(pos2(full.left(), mini.bottom()), full.max);
        let tl_x0 = area.left() + HEAD_W; let tl_w = area.right() - tl_x0;
        let bpm = self.mix.bpm;
        let len = self.mix.length_beats().max(64.0);
        let beat_now = self.beat();
        if ui.rect_contains_pointer(Rect::from_min_max(pos2(tl_x0, area.top()), area.max)) {
            let (zoom, sc, mx) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if zoom != 1.0 { let mxb = mx.map(|m| self.scroll + ((m.x - tl_x0) / self.ppb) as f64).unwrap_or(self.scroll);
                self.ppb = (self.ppb * zoom).clamp(0.3, 80.0); if let Some(m) = mx { self.scroll = mxb - ((m.x - tl_x0) / self.ppb) as f64; } }
            let d = if sc.x.abs() > sc.y.abs() { sc.x } else { sc.y };
            if d != 0.0 && zoom == 1.0 { self.scroll -= (d / self.ppb) as f64; self.follow = false; }
        }
        if self.playing() && self.follow { let px = (beat_now - self.scroll) as f32 * self.ppb; if px > tl_w * 0.85 || px < 0.0 { self.scroll = beat_now - (tl_w * 0.15 / self.ppb) as f64; } }
        self.scroll = self.scroll.clamp(-16.0, len);
        let ppb = self.ppb; let scroll = self.scroll;
        let bx = |b: f64| tl_x0 + ((b - scroll) as f32) * ppb;
        let xb = |x: f32| scroll + ((x - tl_x0) / ppb) as f64;
        let p = ui.painter().clone();

        // minimap / overview
        p.rect_filled(mini, 0.0, PANEL);
        let mw = mini.width() - 20.0; let mxf = |b: f64| mini.left() + 10.0 + (b / len) as f32 * mw;
        p.line_segment([pos2(mini.left() + 10.0, mini.center().y), pos2(mini.right() - 10.0, mini.center().y)], Stroke::new(2.0_f32, C::from_rgb(120, 118, 20)));
        for t in &self.mix.tracks { p.circle_filled(pos2(mxf(t.vis_start(bpm)), mini.center().y), 2.0, ACCENT); }
        p.rect_stroke(Rect::from_min_max(pos2(mxf(scroll), mini.top() + 1.0), pos2(mxf(scroll + (tl_w / ppb) as f64), mini.bottom() - 1.0)), 2.0, Stroke::new(1.5_f32, ACCENT));
        p.rect_filled(Rect::from_center_size(pos2(mxf(beat_now), mini.center().y), vec2(3.0, mini_h - 2.0)), 1.0, ACCENT);
        let mr = ui.interact(mini, ui.id().with("mini"), Sense::click_and_drag());
        if let Some(pp) = mr.interact_pointer_pos() { if mr.dragged() || mr.clicked() { self.scroll = (((pp.x - mini.left() - 10.0) / mw) as f64 * len - (tl_w / ppb) as f64 / 2.0).clamp(-16.0, len); self.follow = false; } }

        let ruler = Rect::from_min_max(area.min, pos2(area.right(), area.top() + RULER_H));
        let mut y = ruler.bottom();
        let mut rows = vec![];
        let deck_h = ((area.bottom() - y - 2.0 * SAMPLE_H - TEMPO_H) / 2.0).clamp(96.0, 230.0);
        for (lane, h) in [(0usize, deck_h), (1, deck_h), (2, SAMPLE_H), (3, SAMPLE_H), (4, TEMPO_H)] { rows.push((lane, Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), y + h)))); y += h; }
        let tlrect = Rect::from_min_max(pos2(tl_x0, ruler.bottom()), pos2(area.right(), y));
        let clip = |r: Rect| Rect::from_min_max(pos2(r.left().max(tl_x0), r.top()), pos2(r.right().min(area.right()), r.bottom()));
        let tp = p.with_clip_rect(Rect::from_min_max(pos2(tl_x0, area.top()), area.max));
        for (lane, r) in &rows { p.rect_filled(*r, 0.0, if *lane < 2 { ROW } else { C::from_rgb(28, 29, 33) }); p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, LINE)); }
        // empty-area click = seek (registered first so clips on top win)
        let bg = ui.interact(tlrect, ui.id().with("tlbg"), Sense::click());
        // grid
        let b0 = xb(tl_x0).floor().max(-16.0) as i64; let b1 = xb(area.right()).ceil() as i64;
        let step = if ppb < 1.5 { 16 } else if ppb < 4.0 { 4 } else { 1 };
        for b in b0..=b1 { if b.rem_euclid(step) != 0 { continue; } let x = bx(b as f64);
            tp.line_segment([pos2(x, ruler.bottom()), pos2(x, y)], Stroke::new(1.0_f32, if b.rem_euclid(4) == 0 { C::from_rgb(44, 46, 52) } else { C::from_rgb(35, 37, 41) })); }
        // ruler
        p.rect_filled(ruler, 0.0, PANEL);
        let sec_px = ppb as f64 * bpm / 60.0;
        let st = [1.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0].into_iter().find(|s| s * sec_px > 70.0).unwrap_or(600.0);
        let mut s = (self.sec(xb(tl_x0)) / st).floor() * st;
        while s < self.sec(xb(area.right())) { let x = bx(s * bpm / 60.0);
            tp.line_segment([pos2(x, ruler.bottom() - 8.0), pos2(x, ruler.bottom())], Stroke::new(1.0_f32, DIM));
            if s >= 0.0 { tp.text(pos2(x + 3.0, ruler.top() + 9.0), Align2::LEFT_CENTER, fmt_t(s), FontId::proportional(12.0), TXT); }
            for k in 1..5 { let xx = bx((s + st / 5.0 * k as f64) * bpm / 60.0); tp.line_segment([pos2(xx, ruler.bottom() - 4.0), pos2(xx, ruler.bottom())], Stroke::new(1.0_f32, LINE)); }
            s += st; }
        let rr = ui.interact(Rect::from_min_max(pos2(tl_x0, ruler.top()), ruler.max), ui.id().with("ruler"), Sense::click_and_drag());
        if let Some(pp) = rr.interact_pointer_pos() { if rr.clicked() || rr.dragged() { self.seek(xb(pp.x)); } }

        // left time display
        let hr = Rect::from_min_max(area.min, pos2(tl_x0, ruler.bottom()));
        p.rect_filled(hr, 0.0, PANEL);
        p.text(pos2(hr.left() + 8.0, hr.center().y), Align2::LEFT_CENTER, "〰", FontId::proportional(14.0), DIM);
        p.text(pos2(hr.left() + 46.0, hr.top() + 10.0), Align2::CENTER_CENTER, fmt_t(self.sec(beat_now)), FontId::proportional(13.0), TXT);
        p.text(pos2(hr.left() + 46.0, hr.top() + 23.0), Align2::CENTER_CENTER, fmt_t(self.sec(self.mix.length_beats())), FontId::proportional(9.0), DIM);
        let order = self.mix.main_order();
        let cur: Vec<usize> = order.iter().copied().filter(|&i| { let t = &self.mix.tracks[i]; beat_now >= t.vis_start(bpm) && beat_now < t.end(bpm) }).collect();
        let pos_n = cur.first().and_then(|c| order.iter().position(|i| i == c)).map(|k| k + 1).unwrap_or(0);
        p.text(pos2(hr.left() + 84.0, hr.top() + 10.0), Align2::CENTER_CENTER, format!("{}/{}", pos_n, order.len()), FontId::proportional(12.0), TXT);
        p.text(pos2(hr.left() + 84.0, hr.top() + 23.0), Align2::CENTER_CENTER, "TRACKS", FontId::proportional(8.0), DIM);
        let kb = Rect::from_min_max(pos2(hr.right() - 26.0, hr.top() + 3.0), pos2(hr.right() - 3.0, hr.bottom() - 3.0));
        p.rect_filled(kb, 2.0, C::from_rgb(36, 62, 92)); p.rect_stroke(kb, 2.0, Stroke::new(1.0_f32, C::from_rgb(90, 170, 230)));
        let ks: Vec<String> = cur.iter().take(2).map(|&i| self.mix.tracks[i].key.clone()).collect();
        let k1 = ks.first().cloned().unwrap_or("–".into()); let k2 = ks.get(1).cloned().unwrap_or(k1.clone());
        p.text(pos2(kb.center().x, kb.top() + 7.0), Align2::CENTER_CENTER, k1, FontId::proportional(9.5), C::from_rgb(140, 210, 250));
        p.text(pos2(kb.center().x, kb.bottom() - 7.0), Align2::CENTER_CENTER, k2, FontId::proportional(9.5), C::from_rgb(140, 210, 250));

        let trans = self.mix.transitions();
        let (d1, d2) = (rows[0].1, rows[1].1);
        let mut clicked_clip: Option<(usize, f64, u8)> = None;
        let mut bm_mode = self.bm_mode; let mut bm_act: Option<(usize, i64, u8)> = None; let mut bm_hover: Option<egui::Pos2> = None;
        for (ti, t) in self.mix.tracks.iter().enumerate() {
            let Some((_, an)) = self.store.get(&t.path) else { continue };
            let row = rows[t.lane.min(3)].1;
            let (vs, ve) = (t.vis_start(bpm), t.end(bpm));
            let cr = Rect::from_min_max(pos2(bx(vs), row.top() + 1.0), pos2(bx(ve), row.bottom() - 1.0));
            if cr.right() < tl_x0 || cr.left() > area.right() { continue; }
            let vis = clip(cr);
            let sample = t.lane >= 2;
            let base = if sample { if t.lane == 2 { C::from_rgb(196, 186, 20) } else { C::from_rgb(70, 90, 190) } } else { OLIVE };
            let title_h = if sample { 11.0 } else { 14.0 };
            let tr = Rect::from_min_max(cr.min, pos2(cr.right(), cr.top() + title_h));
            tp.rect_filled(Rect::from_min_max(pos2(cr.left(), tr.bottom()), cr.max), 0.0, if sample { base.gamma_multiply(0.35) } else { C::from_rgb(24, 25, 29) });
            tp.rect_filled(tr, 0.0, base);
            let mut x = vis.left() + 3.0;
            if !sample { let kr = Rect::from_min_size(pos2(x, tr.top() + 1.5), vec2(20.0, title_h - 3.0));
                tp.rect_filled(kr, 2.0, if t.lane == 0 { C::from_rgb(232, 168, 216) } else { C::from_rgb(170, 236, 150) });
                tp.text(kr.center(), Align2::CENTER_CENTER, &t.key, FontId::proportional(9.0), C::BLACK); x += 24.0; }
            if let Some(pr) = self.stem_prog.get(&t.path) { let v = pr.load(std::sync::atomic::Ordering::Relaxed) as f32 / 10.0; let dl = self.dl_prog.load(std::sync::atomic::Ordering::Relaxed);
                let txt = if v == 0.0 && dl > 0 && dl < 1000 { format!("◌ Downloading stem model {:.0}%", dl as f32 / 10.0) } else { format!("◌ Separating stems {v:.0}%") };
                let w = 190.0; let r = Rect::from_min_size(pos2(vis.right() - w - 4.0, tr.top() + 1.0), vec2(w, title_h - 2.0));
                if r.left() > vis.left() + 60.0 { tp.rect_filled(r, 3.0, C::from_rgb(30, 30, 34)); tp.rect_filled(Rect::from_min_size(r.min, vec2(w * v / 100.0, r.height())), 3.0, C::from_rgb(60, 70, 140));
                    tp.text(r.center(), Align2::CENTER_CENTER, txt, FontId::proportional(9.5), TXT); } }
            else if !sample && self.stems.contains_key(&t.path) { tp.text(pos2(vis.right() - 6.0, tr.center().y), Align2::RIGHT_CENTER, "STEMS", FontId::proportional(8.5), C::from_rgb(200, 220, 255)); }
            tp.text(pos2(x, tr.center().y), Align2::LEFT_CENTER, if sample { t.title.clone() } else { format!("{} - {}", t.artist, t.title) }, FontId::proportional(if sample { 9.5 } else { 10.5 }), if sample && t.lane == 2 { C::BLACK } else { TXT });
            let wave = Rect::from_min_max(pos2(cr.left(), tr.bottom() + if sample { 0.0 } else { 12.0 }), pos2(cr.right(), cr.bottom() - if sample { 0.0 } else { 9.0 }));
            let cps = an.peak_sr as f64 / PEAK_HOP as f64;
            let (cy, hh) = (wave.center().y, wave.height() / 2.0);
            let overlap: Vec<(f64, f64, bool, usize)> = trans.iter().filter(|x| x.0 == ti || x.1 == ti).map(|x| (x.2, x.3, x.0 == ti, x.1)).collect();
            let partner = |o: &(f64, f64, bool, usize)| -> usize { if o.2 { o.3 } else { trans.iter().find(|x| x.1 == ti).map(|x| x.0).unwrap_or(ti) } };
            let my_stems = self.stems.get(&t.path);
            let grid = t.grid();
            let mut px = vis.left().floor();
            while px < vis.right() {
                let (bb0, bb1) = (xb(px), xb(px + 1.0));
                let c0 = (grid.sec(bb0) * cps) as i64;
                let c1 = ((grid.sec(bb1) * cps) as i64).max(c0 + 1);
                if c0 >= 0 && (c0 as usize) < an.peaks.len() {
                    let mut m = [0u8; 3]; for c in c0..c1.min(an.peaks.len() as i64) { let v = an.peaks[c as usize]; for k in 0..3 { m[k] = m[k].max(v[k]); } }
                    let f = |v: u8| v as f32 / 255.0;
                    let ov = if sample { None } else { overlap.iter().find(|o| bb0 >= o.0 && bb0 <= o.1).filter(|o| my_stems.is_some() && self.stems.contains_key(&self.mix.tracks[partner(o)].path)) };
                    if let (Some(o), Some(stm)) = (ov, my_stems) {
                        // real stem lanes inside the transition
                        let lh = wave.height() / 4.0; let modes = self.mix.tracks[o.3].trans.stems;
                        let prog = ((bb0 - o.0) / (o.1 - o.0)) as f32;
                        let mut sm = [0u8; 4]; for c in c0..c1.min(stm.peaks.len() as i64) { let v = stm.peaks[c as usize]; for k in 0..4 { sm[k] = sm[k].max(v[k]); } }
                        for k in 0..4 { let ly = wave.top() + lh * (k as f32 + 0.5); let g = engine::stem_gain(modes[k], prog, o.2);
                            tp.line_segment([pos2(px + 0.5, ly - lh * 0.47), pos2(px + 0.5, ly + lh * 0.47)], Stroke::new(1.0_f32, STEM[k].gamma_multiply(0.08 + 0.12 * g)));
                            let a = (f(sm[k]) * lh * 0.47).max(0.5);
                            tp.line_segment([pos2(px + 0.5, ly - a), pos2(px + 0.5, ly + a)], Stroke::new(1.0_f32, STEM[k].gamma_multiply(0.3 + 0.7 * g))); }
                    } else if sample {
                        let a = f(m[0]).max(f(m[1])) * (wave.height() - 2.0);
                        tp.line_segment([pos2(px + 0.5, wave.bottom() - a), pos2(px + 0.5, wave.bottom())], Stroke::new(1.0_f32, if t.lane == 2 { C::from_rgb(30, 50, 150) } else { C::from_rgb(150, 180, 255) }));
                    } else {
                        let (lo, mi, hi) = (f(m[0]) * hh, f(m[1]) * hh * 0.75, f(m[2]) * hh * 0.45);
                        tp.line_segment([pos2(px + 0.5, cy - lo), pos2(px + 0.5, cy + lo)], Stroke::new(1.0_f32, C::from_rgb(44, 72, 220)));
                        tp.line_segment([pos2(px + 0.5, cy - mi), pos2(px + 0.5, cy + mi)], Stroke::new(1.0_f32, C::from_rgb(50, 186, 96)));
                        tp.line_segment([pos2(px + 0.5, cy - hi), pos2(px + 0.5, cy + hi)], Stroke::new(1.0_f32, C::from_rgb(200, 240, 214)));
                    }
                }
                px += 1.0;
            }
            if !sample && ppb >= 2.5 {
                let k0 = (xb(vis.left()) - t.start_beat).ceil() as i64; let k1 = (xb(vis.right()) - t.start_beat).floor() as i64;
                for k in k0..=k1 { let x = bx(t.start_beat + k as f64); let down = k.rem_euclid(4) == 0;
                    tp.line_segment([pos2(x, wave.top()), pos2(x, wave.bottom())], Stroke::new(if down { 1.2_f32 } else { 1.0 }, C::from_rgba_unmultiplied(255, 255, 255, if down { 70 } else { 16 }))); }
            }
            if !sample { for o in overlap.iter().filter(|o| !o.2) { let p = partner(o);
                let (sc, col, lab) = crate::automix::rate(&self.ao_item(p), &self.ao_item(ti));
                let x0 = bx(o.0).max(vis.left()) + 4.0; let r = Rect::from_min_size(pos2(x0, wave.top() + 2.0), vec2(8.0 + 5.6 * lab.chars().count() as f32, 13.0));
                if r.right() < vis.right() { tp.rect_filled(r, 3.0, C::from_rgba_unmultiplied(20, 21, 24, 210)); tp.circle_filled(pos2(r.left() + 6.0, r.center().y), 3.5, compat_col(col));
                    tp.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, &lab, FontId::proportional(9.5), TXT); let _ = sc; } } }
            if !sample { for o in &overlap {
                if my_stems.is_some() && self.stems.contains_key(&self.mix.tracks[partner(o)].path) { continue; }
                let preset = self.mix.tracks[o.3].trans.preset;
                let (x0, x1) = (bx(o.0).max(vis.left()), bx(o.1).min(vis.right()));
                if x1 > x0 + 1.0 {
                    let n = ((x1 - x0) / 2.0).ceil() as usize;
                    let pts: Vec<egui::Pos2> = (0..=n).map(|k| { let x = x0 + (x1 - x0) * k as f32 / n as f32;
                        let pr = ((xb(x) - o.0) / (o.1 - o.0)) as f32; let g = engine::preset_gain(preset, pr, o.2);
                        pos2(x, wave.bottom() - 2.0 - g * (wave.height() - 4.0)) }).collect();
                    tp.add(egui::Shape::line(pts, Stroke::new(1.5_f32, C::from_rgb(235, 235, 240))));
                }
            } }
            if !sample {
                let every = if ppb * 4.0 > 40.0 { 1 } else if ppb * 4.0 > 10.0 { 4 } else { 16 };
                let (mut b, mut bar) = (t.start_beat, 1);
                while b < ve { if (bar - 1) % every == 0 { let x = bx(b); if x > vis.left() && x < vis.right() {
                    tp.line_segment([pos2(x, tr.bottom()), pos2(x, tr.bottom() + 10.0)], Stroke::new(1.0_f32, DIM));
                    tp.text(pos2(x + 2.0, tr.bottom() + 5.0), Align2::LEFT_CENTER, format!("{bar}"), FontId::proportional(8.5), DIM); } }
                    b += 4.0; bar += 1; }
                let names = ["DOWN", "CHORUS", "UP"]; let cols = [C::from_rgb(150, 110, 40), C::from_rgb(40, 170, 60), C::from_rgb(140, 70, 200)];
                for (k, &(sb, kind)) in an.sections.iter().enumerate() {
                    let end = an.sections.get(k + 1).map(|x| x.0).unwrap_or(ve - t.start_beat);
                    let r = Rect::from_min_max(pos2(bx(t.start_beat + sb), cr.bottom() - 8.0), pos2(bx(t.start_beat + end) - 1.0, cr.bottom()));
                    tp.rect_filled(r, 0.0, cols[kind as usize]);
                    if r.width() > 40.0 { tp.text(pos2(r.left().max(tl_x0) + 3.0, r.center().y), Align2::LEFT_CENTER, names[kind as usize], FontId::proportional(7.5), C::BLACK); }
                }
                for o in &overlap { let x1 = bx(o.1).min(cr.right());
                    for k in 0..4 { let r = Rect::from_min_size(pos2(x1 - 9.0 * (4 - k) as f32 - 3.0, cr.bottom() - 16.0), vec2(7.0, 6.0));
                        tp.rect_filled(r, 1.5, STEM[k].gamma_multiply(0.7)); } }
            }
            let selected = self.sel == Some(ti);
            tp.rect_stroke(cr, 2.0, Stroke::new(if selected { 1.5_f32 } else { 0.5 }, if selected { ACCENT } else { C::from_rgb(70, 70, 40) }));
            let resp = ui.interact(vis, ui.id().with(("clip", ti)), Sense::click_and_drag());
            if let Some(pp) = resp.interact_pointer_pos() {
                let (alt, shift) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
                if resp.drag_started() && alt && !sample { clicked_clip = Some((ti, xb(pp.x), 2u8)); }
                else if resp.drag_started() && !self.lock { clicked_clip = Some((ti, xb(pp.x), 1)); }
                else if resp.clicked() && shift && !sample { clicked_clip = Some((ti, xb(pp.x), 3)); }
                else if resp.clicked() { clicked_clip = Some((ti, xb(pp.x), 0)); }
            }
            if !sample && ppb >= 22.0 {
                let k0 = (xb(vis.left()) - t.start_beat).ceil() as i64; let k1 = (xb(vis.right()) - t.start_beat).floor() as i64;
                for k in k0..=k1 {
                    let x = bx(t.start_beat + k as f64);
                    let hr = Rect::from_center_size(pos2(x, wave.top() + 7.0), vec2(12.0, 14.0));
                    let r = ui.interact(hr, ui.id().with(("bm", ti, k)), Sense::click_and_drag());
                    let hot = r.hovered() || r.dragged() || self.bm_drag.as_ref().map_or(false, |d| d.ti == ti && d.k == k);
                    let col = if hot { bm_col(bm_mode) } else if k.rem_euclid(4) == 0 { C::from_rgb(235, 235, 240) } else { C::from_rgb(150, 150, 160) };
                    tp.add(egui::Shape::convex_polygon(vec![pos2(x - 5.0, wave.top() + 1.0), pos2(x + 5.0, wave.top() + 1.0), pos2(x, wave.top() + 9.0)], col, Stroke::NONE));
                    if hot { tp.line_segment([pos2(x, wave.top()), pos2(x, wave.bottom())], Stroke::new(1.5_f32, col)); }
                    if r.hovered() { bm_hover = Some(pos2(x, wave.top())); ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal); }
                    if r.drag_started() { bm_act = Some((ti, k, 0)); }
                    r.context_menu(|ui| {
                        if ui.button("Set downbeat here").clicked() { bm_act = Some((ti, k, 1)); ui.close_menu(); }
                        if ui.button("Set first beat here").clicked() { bm_act = Some((ti, k, 2)); ui.close_menu(); }
                        ui.separator();
                        for m in BmMode::ALL { ui.radio_value(&mut bm_mode, m, m.name()); }
                    });
                }
            }
        }
        // transition boxes spanning both decks
        let mut clicked_trans = None;
        for (k, &(o, i, s, e)) in trans.iter().enumerate() {
            let r = Rect::from_min_max(pos2(bx(s), d1.top() + 1.0), pos2(bx(e), d2.bottom() - 1.0));
            if r.right() < tl_x0 || r.left() > area.right() { continue; }
            let sel = self.sel_in == Some(i);
            tp.rect_filled(r, 0.0, C::from_rgba_unmultiplied(84, 104, 236, if sel { 30 } else { 16 }));
            tp.rect_stroke(r, 0.0, Stroke::new(if sel { 2.0_f32 } else { 1.2 }, BLUE));
            let bar = Rect::from_min_max(r.min, pos2(r.right(), r.top() + 14.0));
            tp.rect_filled(bar, 0.0, BLUE);
            let tt = &self.mix.tracks[i].trans; let nst = tt.stems.iter().filter(|m| **m != StemMode::Crossfade).count();
            let label = if nst > 0 { format!("{}  {} · {} Stems", k + 1, tt.preset.name(), nst) } else { format!("{}  {}", k + 1, tt.preset.name()) };
            tp.text(pos2(clip(bar).left() + 4.0, bar.center().y), Align2::LEFT_CENTER, label, FontId::proportional(10.0), C::WHITE);
            if bar.width() > 170.0 {
                let br = Rect::from_min_max(pos2(bar.right() - 72.0, bar.top() + 1.5), pos2(bar.right() - 18.0, bar.bottom() - 1.5));
                tp.rect_filled(br, 3.0, C::from_rgb(140, 200, 250));
                tp.text(br.center(), Align2::CENTER_CENTER, format!("{} → {}", self.mix.tracks[o].key, self.mix.tracks[i].key), FontId::proportional(9.0), C::BLACK);
                tp.text(pos2(bar.right() - 9.0, bar.center().y), Align2::CENTER_CENTER, "⚙", FontId::proportional(9.0), C::WHITE);
                let lr = Rect::from_min_size(pos2(bar.right() - 90.0, bar.top() + 2.0), vec2(12.0, 10.0)); tp.rect_filled(lr, 2.0, C::WHITE);
                tp.text(lr.center(), Align2::CENTER_CENTER, "≡", FontId::proportional(8.0), BLUE);
            }
            let resp = ui.interact(clip(bar), ui.id().with(("tr", k)), Sense::click());
            if resp.clicked() { clicked_trans = Some(i); }
            resp.on_hover_text(format!("Transition {}: {} ({} bars). Click for settings.", k + 1, tt.preset.name(), ((e - s) / 4.0).round()));
        }
        let trow = rows[4].1;
        tp.line_segment([pos2(tl_x0, trow.center().y + 3.0), pos2(area.right(), trow.center().y + 3.0)], Stroke::new(1.5_f32, C::from_rgb(220, 140, 30)));
        tp.text(pos2(tl_x0 + 4.0, trow.center().y - 4.0), Align2::LEFT_CENTER, format!("{:.1} BPM", bpm), FontId::proportional(9.0), C::from_rgb(220, 140, 30));
        if bg.clicked() { if let Some(pp) = bg.interact_pointer_pos() { self.seek(xb(pp.x)); self.sel = None; } }
        if let Some(i) = clicked_trans { self.sel_in = Some(i); self.tab = Tab::Transition; }
        else if let Some((ti, b, drag)) = clicked_clip {
            self.sel = Some(ti);
            if drag == 1 { self.snapshot(); self.drag = Some((ti, b, self.mix.tracks[ti].start_beat)); }
            else if drag == 2 { self.snapshot(); self.grid_drag = Some((ti, ui.input(|i| i.pointer.hover_pos()).map(|p| p.x).unwrap_or(0.0))); }
            else if drag == 3 { self.snapshot(); let t = &self.mix.tracks[ti]; let line = t.start_beat + (b - t.start_beat).round(); self.set_downbeat_at(ti, line); self.status = "Downbeat set".into(); }
            else { self.seek(b); if let Some(x) = trans.iter().find(|x| x.1 == ti || x.0 == ti) { self.sel_in = Some(x.1); } }
        }
        self.bm_mode = bm_mode;
        if let Some(p) = bm_hover { self.bm_pop = Some((p, std::time::Instant::now())); }
        if let Some((p, at)) = self.bm_pop {
            let a = egui::Area::new(egui::Id::new("bm_pop")).order(egui::Order::Foreground).fixed_pos(pos2(p.x - 120.0, p.y - 30.0)).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| { ui.horizontal(|ui| { for m in BmMode::ALL { ui.selectable_value(&mut self.bm_mode, m, m.name()).on_hover_text(m.help()); } }); });
            });
            let keep = a.response.rect.expand(6.0).contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
            if keep || self.bm_drag.is_some() { self.bm_pop = Some((p, if keep { std::time::Instant::now() } else { at })); }
            else if at.elapsed().as_secs_f32() > 0.7 { self.bm_pop = None; }
        }
        if let Some((ti, k, what)) = bm_act {
            self.snapshot(); self.sel = Some(ti);
            match what {
                0 => { let t = &mut self.mix.tracks[ti]; t.ensure_map(); let i = t.map_index(k); let m = t.map.as_ref().unwrap();
                    let pivot = i.saturating_sub(4); let span = m.beats[i] - m.beats[pivot];
                    self.bm_drag = Some(BmDrag { ti, k, i, pivot, span, lastx: ui.input(|i| i.pointer.hover_pos()).map(|p| p.x).unwrap_or(0.0), mode: self.bm_mode }); }
                1 => { let b = self.mix.tracks[ti].start_beat + k as f64; self.set_downbeat_at(ti, b); self.status = "Downbeat set".into(); }
                _ => { let t = &mut self.mix.tracks[ti]; t.ensure_map(); let i = t.map_index(k); let mb = t.start_beat + k as f64;
                    let sec = t.map.as_ref().unwrap().beats[i];
                    let mut m = (**t.map.as_ref().unwrap()).clone(); m.trim_before(i); t.map = Some(Arc::new(m));
                    t.offset = sec; t.start_beat = (mb / 4.0).round() * 4.0; t.normalize_grid(); self.status = "First beat set".into(); }
            }
        }
        if let Some(d) = self.bm_drag.clone() {
            if ui.input(|i| i.pointer.primary_down()) && d.ti < self.mix.tracks.len() {
                if let Some(pp) = ui.input(|i| i.pointer.hover_pos()) { if pp.x != d.lastx {
                    let t = &mut self.mix.tracks[d.ti];
                    let m = t.map.as_ref().unwrap(); let per = if d.i + 1 < m.beats.len() { m.beats[d.i + 1] - m.beats[d.i] } else { 60.0 / t.bpm };
                    let dsec = ((pp.x - d.lastx) / ppb) as f64 * per;
                    let msg = match d.mode {
                        BmMode::Grid => { t.edit_map(|m| m.shift(dsec)); let off = t.map.as_ref().unwrap().beats[d.i] - (d.span + 0.0) * 0.0; format!("Grid moved (beat at {off:.3} s)") }
                        BmMode::Single => { t.edit_map(|m| m.move_single(d.i, dsec)); format!("Beat moved to {:.3} s", t.map.as_ref().unwrap().beats[d.i]) }
                        BmMode::Following => { t.edit_map(|m| m.stretch_following(d.pivot, d.i, dsec)); let m = t.map.as_ref().unwrap();
                            let pct = if d.pivot < d.i { (d.span / (m.beats[d.i] - m.beats[d.pivot]) - 1.0) * 100.0 } else { 0.0 };
                            ui.painter().text(pp + vec2(12.0, -14.0), Align2::LEFT_BOTTOM, format!("{pct:+.2} %"), FontId::proportional(12.0), bm_col(BmMode::Following));
                            format!("Following beats stretched: tempo {pct:+.2} % (anchored 1 bar to the left)") }
                    };
                    self.status = msg; self.bm_drag = Some(BmDrag { lastx: pp.x, ..d });
                } }
            } else { self.bm_drag = None; }
        }
        if let Some((ti, lastx)) = self.grid_drag {
            if ui.input(|i| i.pointer.primary_down()) { if let Some(pp) = ui.input(|i| i.pointer.hover_pos()) {
                let db = ((pp.x - lastx) / ppb) as f64; if ti < self.mix.tracks.len() { let bpm_t = self.mix.tracks[ti].bpm; self.nudge(ti, db * 60.0 / bpm_t); } self.grid_drag = Some((ti, pp.x)); } }
            else { self.grid_drag = None; }
        }
        if let Some((ti, grab, orig)) = self.drag {
            if ui.input(|i| i.pointer.primary_down()) { if let Some(pp) = ui.input(|i| i.pointer.hover_pos()) {
                let nb = self.snap_beat(orig + xb(pp.x) - grab); if ti < self.mix.tracks.len() { self.mix.tracks[ti].start_beat = nb; } } }
            else { self.drag = None; if self.undo.last().is_some_and(|u| *u == serde_json::to_string(&self.mix).unwrap()) { self.undo.pop(); } }
        }
        // zoom buttons on top of timeline
        let mut zx = area.right() - 6.0;
        for (t, act) in [("»", 0), ("«", 1), ("+", 2), ("−", 3), ("⤢", 4)] {
            let r = Rect::from_min_max(pos2(zx - 22.0, ruler.bottom() + 4.0), pos2(zx, ruler.bottom() + 22.0)); zx -= 26.0;
            if r.left() < tl_x0 { break; }
            if Self::small_btn(ui, ui.id().with(("zb", act)), r, t, false, ACCENT) { match act {
                0 => self.scroll += (tl_w / ppb * 0.5) as f64, 1 => self.scroll -= (tl_w / ppb * 0.5) as f64,
                2 => self.ppb = (self.ppb * 1.4).min(80.0), 3 => self.ppb = (self.ppb / 1.4).max(0.3),
                _ => { self.ppb = (tl_w / len as f32).max(0.3); self.scroll = 0.0; } } }
        }
        let phx = bx(beat_now);
        if phx >= tl_x0 && phx <= area.right() {
            tp.line_segment([pos2(phx, ruler.top() + 12.0), pos2(phx, y)], Stroke::new(1.5_f32, C::WHITE));
            tp.add(egui::Shape::convex_polygon(vec![pos2(phx - 6.0, ruler.top() + 6.0), pos2(phx + 6.0, ruler.top() + 6.0), pos2(phx + 6.0, ruler.top() + 16.0), pos2(phx, ruler.top() + 22.0), pos2(phx - 6.0, ruler.top() + 16.0)], C::from_rgb(220, 220, 224), Stroke::NONE));
        }
        if self.out._stream.is_none() { p.text(pos2(tl_x0 + 8.0, y + 12.0), Align2::LEFT_TOP, &self.out.info, FontId::proportional(11.0), C::from_rgb(230, 120, 80)); }
        if self.mix.tracks.is_empty() { p.text(tlrect.center(), Align2::CENTER_CENTER, "Click + or drop audio files here to start mixing", FontId::proportional(16.0), DIM); }
        // deck headers
        for (lane, r) in rows.clone() {
            let h = Rect::from_min_max(r.min, pos2(tl_x0, r.bottom()));
            p.rect_filled(h, 0.0, PANEL); p.line_segment([h.left_bottom(), h.right_bottom()], Stroke::new(1.0_f32, LINE)); p.line_segment([h.right_top(), h.right_bottom()], Stroke::new(1.0_f32, LINE));
            if lane == 4 { p.text(pos2(h.left() + 8.0, h.center().y), Align2::LEFT_CENTER, "›  Tempo lane", FontId::proportional(11.0), TXT); continue; }
            let deck = lane < 2;
            let name = if deck { format!("⏷  Deck {}", lane + 1) } else { format!("›  Sample {}", lane - 1) };
            let ty = if deck { h.top() + 12.0 } else { h.center().y };
            p.text(pos2(h.left() + 6.0, ty), Align2::LEFT_CENTER, name, FontId::proportional(11.0), TXT);
            let meter = Shared::get(&self.out.shared.deck_meter[lane]);
            let bw = 15.0;
            let fr = Rect::from_min_max(pos2(h.left() + if deck { 60.0 } else { 70.0 }, ty - 6.0), pos2(h.right() - if deck { bw + 8.0 } else { 2.0 * bw + 10.0 }, ty + 6.0));
            let d = &mut self.mix.decks[lane];
            Self::fader(ui, ui.id().with(("fader", lane)), fr, &mut d.vol, meter);
            let sr_ = if deck { Rect::from_min_size(pos2(h.right() - bw - 3.0, h.top() + 5.0), vec2(bw, 14.0)) } else { Rect::from_min_size(pos2(h.right() - 2.0 * bw - 6.0, ty - 7.0), vec2(bw, 14.0)) };
            if Self::small_btn(ui, ui.id().with(("solo", lane)), sr_, "S", d.solo, ACCENT) { d.solo = !d.solo; }
            let mr_ = if deck { sr_.translate(vec2(0.0, 17.0)) } else { sr_.translate(vec2(bw + 3.0, 0.0)) };
            if Self::small_btn(ui, ui.id().with(("mute", lane)), mr_, "M", d.mute, C::from_rgb(230, 90, 60)) { d.mute = !d.mute; }
            if deck {
                Self::small_btn(ui, ui.id().with(("aut", lane)), mr_.translate(vec2(0.0, 17.0)), "A", false, ACCENT);
                let ky = h.top() + 44.0; let mut x = h.left() + 16.0;
                for (k, lab) in ["High", "Mid", "Low", "Filter"].iter().enumerate() {
                    let v = match k { 0 => &mut d.hi, 1 => &mut d.mid, 2 => &mut d.low, _ => &mut d.filter };
                    Self::knob(ui, ui.id().with(("knob", lane, k)), pos2(x, ky), v, lab); x += 26.0;
                }
                let sa = Rect::from_min_size(pos2(h.left() + 6.0, h.bottom() - 22.0), vec2(HEAD_W - 30.0, 16.0));
                p.rect_filled(sa, 3.0, C::from_rgb(48, 50, 56)); p.text(pos2(sa.left() + 6.0, sa.center().y), Align2::LEFT_CENTER, "Show all", FontId::proportional(10.0), TXT);
                p.text(pos2(sa.right() - 8.0, sa.center().y), Align2::CENTER_CENTER, "⏷", FontId::proportional(10.0), TXT);
            }
        }
        if y < area.bottom() { p.rect_filled(Rect::from_min_max(pos2(area.left(), y), area.max), 0.0, BG); }
    }
}

impl App {
    fn bottom(&mut self, ui: &mut egui::Ui) {
        let dim = |s: &str| egui::RichText::new(s.to_string()).color(DIM);
        match self.tab {
            Tab::Playlist => self.playlist(ui),
            Tab::Library => self.library_tab(ui),
            Tab::Transition => self.transition_tab(ui),
            Tab::Track => self.track_tab(ui),
            Tab::Zoom => { ui.heading("Zoom"); ui.add(egui::Slider::new(&mut self.ppb, 0.3..=80.0).logarithmic(true).text("pixels per beat"));
                if ui.button("Fit whole mix").clicked() { self.ppb = (ui.available_width() / self.mix.length_beats().max(64.0) as f32).max(0.3); self.scroll = 0.0; }
                ui.label(dim("Tip: Ctrl + scroll zooms around the pointer, scroll moves in time.")); }
            Tab::Video => { ui.heading("Video"); ui.label(dim("Video is not supported in v1.")); }
            Tab::Effects => { ui.heading("Effects"); ui.label(dim("Per deck: 3-band EQ and filter (knobs on the left). Echo is in the \"Echo out\" transition. More effects in v2."));
                for d in 0..2 { ui.horizontal(|ui| { ui.label(format!("Deck {} filter", d + 1)); ui.add(egui::Slider::new(&mut self.mix.decks[d].filter, -1.0..=1.0).text("LP ← → HP")); }); } }
            Tab::Master => { ui.heading("Master"); let mut b = self.mix.bpm;
                ui.horizontal(|ui| { ui.label("Mix tempo"); if ui.add(egui::DragValue::new(&mut b).range(60.0..=200.0).speed(0.1).suffix(" BPM")).changed() { self.mix.bpm = b; } });
                ui.add(egui::Slider::new(&mut self.mix.master, 0.0..=1.5).text("Master volume"));
                ui.checkbox(&mut self.mix.key_lock, "Key lock (keep pitch when tempo changes)").on_hover_text("On: WSOLA time-stretch keeps the pitch. Off: varispeed, pitch follows tempo like a turntable.");
                ui.horizontal(|ui| { ui.checkbox(&mut self.mix.metronome, "Metronome"); ui.add(egui::Slider::new(&mut self.mix.metro_vol, 0.0..=1.0).text("click volume")); });
                ui.label(dim("All tracks play at the mix tempo. The metronome is never included in exports.")); }
            Tab::Samples => { ui.heading("Samples"); ui.horizontal(|ui| {
                if ui.button("Add sample to Sample 1 at playhead…").clicked() { self.pick_files(Some(2)); }
                if ui.button("Add sample to Sample 2 at playhead…").clicked() { self.pick_files(Some(3)); } });
                ui.label(dim("Samples sync to the mix tempo and can be dragged in the timeline."));
                for t in self.mix.tracks.iter().filter(|t| t.lane >= 2) { ui.label(format!("Sample {}: {} ({:.1} BPM, bar {:.0})", t.lane - 1, t.title, t.bpm, t.start_beat / 4.0 + 1.0)); } }
        }
    }

    fn playlist(&mut self, ui: &mut egui::Ui) {
        let order = self.mix.main_order();
        let mut act: Option<(usize, i32)> = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("pl").striped(true).num_columns(11).spacing(vec2(22.0, 6.0)).show(ui, |ui| {
                for h in ["#", "", "TITLE", "ARTIST", "BPM", "KEY", "LENGTH", "DECK", "START", "NEXT TRANSITION", ""] { ui.label(egui::RichText::new(h).size(10.0).color(DIM)); }
                ui.end_row();
                for (k, &i) in order.iter().enumerate() {
                    let t = &self.mix.tracks[i]; let sel = self.sel == Some(i);
                    ui.label(format!("{}", k + 1));
                    if ui.selectable_label(t.locked, if t.locked { "🔒" } else { "🔓" }).on_hover_text("Lock: Auto-order keeps this track at its position").clicked() { act = Some((i, 7)); }
                    if ui.selectable_label(sel, egui::RichText::new(&t.title).color(if sel { ACCENT } else { TXT })).clicked() { act = Some((i, 0)); }
                    ui.label(&t.artist); ui.label(format!("{:.1}", t.bpm));
                    let kc = key_col(&t.key); let (r, _) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::hover());
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0_f32, kc)); ui.painter().text(r.center(), Align2::CENTER_CENTER, &t.key, FontId::proportional(11.0), kc);
                    ui.label(fmt_t(t.duration)); ui.label(format!("{}", t.lane + 1)); ui.label(format!("bar {:.0}", t.start_beat / 4.0 + 1.0));
                    if let Some(&nx) = order.get(k + 1) { let (sc, col, lab) = crate::automix::rate(&self.ao_item(i), &self.ao_item(nx));
                        ui.horizontal(|ui| { let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover()); ui.painter().circle_filled(r.center(), 4.5, compat_col(col));
                            ui.label(egui::RichText::new(lab).size(11.0)).on_hover_text(format!("Compatibility {:.0}%", sc * 100.0)); }); } else { ui.label(""); }
                    ui.horizontal(|ui| { if ui.small_button("⏶").clicked() { act = Some((i, -1)); } if ui.small_button("⏷").clicked() { act = Some((i, 1)); } if ui.small_button("✖").clicked() { act = Some((i, 9)); } });
                    ui.end_row();
                }
            });
            if order.is_empty() { ui.label(egui::RichText::new("No tracks yet. Click + (Add tracks) or drop audio files on the window.").color(DIM)); }
        });
        if let Some((i, a)) = act { match a {
            0 => { self.sel = Some(i); self.seek(self.mix.tracks[i].vis_start(self.mix.bpm)); }
            9 => { self.sel = Some(i); self.remove_sel(); }
            7 => { self.snapshot(); self.mix.tracks[i].locked = !self.mix.tracks[i].locked; }
            d => { let k = order.iter().position(|&x| x == i).unwrap() as i32; let n = k + d;
                if n >= 0 && (n as usize) < order.len() { self.snapshot(); let mut o = order.clone(); o.swap(k as usize, n as usize); self.mix.relayout(&o, 16.0); } }
        } }
    }

    fn transition_tab(&mut self, ui: &mut egui::Ui) {
        let trans = self.mix.transitions();
        let k = self.sel_in.and_then(|i| trans.iter().position(|x| x.1 == i)).unwrap_or(0);
        let Some(&(o, i, s, e)) = trans.get(k) else { ui.label(egui::RichText::new("No transition yet. Add at least two tracks.").color(DIM)); return; };
        self.sel_in = Some(i);
        ui.horizontal(|ui| {
            let (a, b) = (&self.mix.tracks[o], &self.mix.tracks[i]);
            ui.label(format!("{}:", k + 1));
            for (n, t) in [(0, a), (1, b)] {
                if n == 1 { ui.label("→"); }
                ui.label(egui::RichText::new(format!(" {} ", t.key)).background_color(C::from_rgb(80, 170, 140)).color(C::BLACK));
                ui.label(egui::RichText::new(&t.title).strong()); ui.label(egui::RichText::new(format!("- {}", t.artist)).color(DIM));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cur = ((e - s) / 4.0).round() as i64;
                for bars in [32i64, 16, 8, 4] { if ui.selectable_label(cur == bars, format!("{bars}")).on_hover_text("Overlap length in bars").clicked() {
                    self.snapshot(); let delta = (bars - cur) as f64 * 4.0; let ord = self.mix.main_order(); let pos = ord.iter().position(|&x| x == i).unwrap();
                    for &j in &ord[pos..] { self.mix.tracks[j].start_beat -= delta; } } }
                ui.label(egui::RichText::new("Bars:").color(DIM));
            });
        });
        ui.add_space(4.0);
        let h = ui.available_height() - 2.0;
        ui.horizontal_top(|ui| {
            let mut preset = self.mix.tracks[i].trans.preset; let mut changed = false;
            egui::Frame::none().fill(C::from_rgb(40, 42, 47)).rounding(6.0).inner_margin(8.0).show(ui, |ui| { ui.set_width(130.0); ui.set_height(h - 16.0); ui.vertical(|ui| {
                ui.horizontal(|ui| { ui.label(egui::RichText::new("Presets").strong()); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { let _ = ui.small_button("Save"); }); });
                ui.add_space(4.0);
                for p in Preset::ALL { let on = preset == p;
                    if ui.add(egui::Button::new(egui::RichText::new(p.name()).color(if on { ACCENT } else { TXT })).frame(on).fill(if on { C::from_rgb(60, 58, 20) } else { C::TRANSPARENT }).stroke(if on { Stroke::new(1.0_f32, ACCENT) } else { Stroke::NONE }).min_size(vec2(118.0, 20.0))).clicked() { preset = p; changed = true; } }
            }); });
            egui::Frame::none().fill(C::from_rgb(52, 54, 60)).rounding(6.0).inner_margin(8.0).show(ui, |ui| { ui.set_width(130.0); ui.set_height(h - 16.0); ui.vertical(|ui| {
                ui.label(egui::RichText::new("Categories").strong()); ui.add_space(4.0);
                for (n, c) in [("☰ Equalizer", ""), ("〰 Effects", ""), ("🎸 Stems", "4"), ("✨ My presets", "0")] {
                    ui.horizontal(|ui| { let on = n.contains("Stems"); ui.add(egui::Button::new(n).frame(on).min_size(vec2(96.0, 20.0)));
                        if !c.is_empty() { ui.label(egui::RichText::new(format!(" {c} ")).color(if on { C::BLACK } else { DIM }).background_color(if on { ACCENT } else { C::TRANSPARENT })); } });
                }
            }); });
            if changed { self.snapshot(); self.mix.tracks[i].trans.preset = preset;
                self.mix.tracks[i].trans.stems = match preset { Preset::BassSwap => [StemMode::Crossfade, StemMode::Swap, StemMode::Crossfade, StemMode::Crossfade],
                    Preset::EchoOut => [StemMode::Swap; 4], _ => [StemMode::Crossfade; 4] }; }
            let mut stems = self.mix.tracks[i].trans.stems; let mut sch = false;
            for k in 0..4 {
                egui::Frame::none().fill(C::from_rgb(40, 42, 47)).rounding(6.0).show(ui, |ui| { ui.set_width(118.0); ui.set_height(h); ui.vertical(|ui| {
                    let (hr, _) = ui.allocate_exact_size(vec2(118.0, 24.0), Sense::hover());
                    ui.painter().rect_filled(hr, Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, STEM[k]);
                    ui.painter().text(pos2(hr.left() + 8.0, hr.center().y), Align2::LEFT_CENTER, STEM_N[k], FontId::proportional(12.0), C::WHITE);
                    ui.add_space(4.0);
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for m in StemMode::ALL { let on = stems[k] == m;
                        ui.horizontal(|ui| { ui.add_space(6.0); if ui.add(egui::Button::new(egui::RichText::new(m.name()).color(TXT)).frame(on).fill(C::TRANSPARENT).stroke(if on { Stroke::new(1.0_f32, ACCENT) } else { Stroke::NONE }).min_size(vec2(104.0, 20.0))).clicked() { stems[k] = m; sch = true; } }); }
                }); });
            }
            if sch { self.snapshot(); self.mix.tracks[i].trans.stems = stems; }
            ui.vertical(|ui| { ui.add_space(6.0); let both = self.stems.contains_key(&self.mix.tracks[o].path) && self.stems.contains_key(&self.mix.tracks[i].path);
                ui.label(egui::RichText::new(if both { "Stems ready: the Drums/Bass/Melody/Vocals\nchoices drive the audio in this overlap.\nThe preset adds its EQ/filter/echo moves." } else { "Separate stems for both tracks to make\nthe stem choices drive the audio.\nUntil then the preset's volume curve is used." }).color(DIM).size(11.0));
                if !both && ui.button("Separate stems for both").clicked() { self.request_stems(o); self.request_stems(i); }
                if ui.button("▶ Listen to transition").clicked() { self.seek(s - 8.0); let _ = self.out.tx.send(Cmd::Play(true)); } });
        });
    }

    fn track_tab(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.sel.filter(|&i| i < self.mix.tracks.len()) else { ui.label(egui::RichText::new("Select a track in the timeline or playlist.").color(DIM)); return; };
        let bpm_mix = self.mix.bpm;
        let mut remove = false; let mut sep = false;
        ui.horizontal_top(|ui| {
        ui.vertical(|ui| { ui.set_width(520.0);
            let t = &mut self.mix.tracks[i];
            ui.heading(format!("{} — {}", t.title, t.artist));
            egui::Grid::new("tt").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label("Track BPM"); ui.add(egui::DragValue::new(&mut t.bpm).range(60.0..=200.0).speed(0.01).fixed_decimals(2)); ui.end_row();
                ui.label("First beat (s)"); ui.add(egui::DragValue::new(&mut t.offset).range(0.0..=30.0).speed(0.002).fixed_decimals(3)); ui.end_row();
                ui.label("Key (Camelot)"); ui.text_edit_singleline(&mut t.key); ui.end_row();
                ui.label("Start (bar)"); let mut bar = t.start_beat / 4.0 + 1.0; if ui.add(egui::DragValue::new(&mut bar).speed(0.1).fixed_decimals(0)).changed() { t.start_beat = (bar.round() - 1.0) * 4.0; } ui.end_row();
                ui.label("Tempo change"); ui.label(format!("{:+.1}% (plays at {:.1} BPM)", (bpm_mix / t.bpm - 1.0) * 100.0, bpm_mix)); ui.end_row();
                ui.label("Beatgrid"); if t.grid().dynamic() { let (lo, hi) = t.map.as_ref().map(|m| m.tempo_range()).unwrap_or((t.bpm, t.bpm));
                    ui.label(format!("Beat map, follows tempo {lo:.2}–{hi:.2} BPM")).on_hover_text("Every beat is tracked, so tempo drift stays locked to the mix grid. Editing the BPM by hand switches to a fixed grid; Reset to analysis brings the beat map back."); }
                else { ui.label("Fixed (linear)"); } ui.end_row();
                ui.label("File"); ui.label(egui::RichText::new(&t.path).color(DIM).size(10.0)); ui.end_row();
            });
            if ui.button("Remove track").clicked() { remove = true; }
            ui.add_space(6.0);
            let path = self.mix.tracks[i].path.clone();
            if self.stems.contains_key(&path) { ui.label(egui::RichText::new("✔ Stems separated (drums, bass, melody, vocals)").color(C::from_rgb(120, 200, 140))); }
            else if let Some(p) = self.stem_prog.get(&path) { ui.horizontal(|ui| { ui.spinner(); ui.label(format!("Separating stems… {:.0}%", p.load(std::sync::atomic::Ordering::Relaxed) as f32 / 10.0)); }); }
            else { if ui.button("Separate stems").on_hover_text("Run HT-Demucs on this track (GPU if available). The model downloads once (~166 MB).").clicked() { sep = true; }
                if let Some(e) = self.stem_err.get(&path) { ui.label(egui::RichText::new(e).color(C::from_rgb(230, 120, 80)).size(11.0)); } }
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("Beatgrid").strong());
            ui.horizontal_wrapped(|ui| {
            for (lab, ms) in [("−10 ms", -10.0), ("−1 ms", -1.0), ("+1 ms", 1.0), ("+10 ms", 10.0)] {
                if ui.button(lab).on_hover_text("Nudge the grid relative to the audio").clicked() { self.snapshot(); self.nudge(i, ms / 1000.0); } }
            if ui.button("Set downbeat at playhead").on_hover_text("The beat under the playhead becomes bar 1 and lands on the nearest mix bar. Shortcut: Shift+click a beat line on the clip.").clicked() {
                self.snapshot(); let b = self.beat(); let t = &self.mix.tracks[i]; let line = t.start_beat + (b - t.start_beat).round(); self.set_downbeat_at(i, line); }
            if ui.button("BPM ×2").clicked() { self.snapshot(); self.mix.tracks[i].bpm = (self.mix.tracks[i].bpm * 2.0).min(220.0); }
            if ui.button("BPM ÷2").clicked() { self.snapshot(); self.mix.tracks[i].bpm = (self.mix.tracks[i].bpm / 2.0).max(40.0); }
            let now = std::time::Instant::now();
            if ui.button("Tap tempo").on_hover_text("Tap at least 4 times in time with the track").clicked() {
                if self.taps.last().is_some_and(|l| now.duration_since(*l).as_secs_f64() > 2.0) { self.taps.clear(); }
                self.taps.push(now); if self.taps.len() > 16 { self.taps.remove(0); }
                if self.taps.len() >= 4 { let span = self.taps.last().unwrap().duration_since(self.taps[0]).as_secs_f64(); let bpm = 60.0 * (self.taps.len() - 1) as f64 / span;
                    if (40.0..=220.0).contains(&bpm) { if self.taps.len() == 4 { self.snapshot(); } self.mix.tracks[i].bpm = (bpm * 10.0).round() / 10.0; } }
            }
            if !self.taps.is_empty() { ui.label(egui::RichText::new(format!("{} taps", self.taps.len())).color(DIM)); }
            if ui.button("Reset to analysis").clicked() { if let Some((_, an)) = self.store.get(&self.mix.tracks[i].path).cloned() { self.snapshot();
                let t = &mut self.mix.tracks[i]; let anchor = t.start_beat - t.intro_beats(); t.bpm = an.bpm; t.offset = an.offset; t.map = an.beat_map();
                t.start_beat = ((anchor + t.intro_beats()) / 4.0).round() * 4.0; } }
            });
            ui.label(egui::RichText::new("On the clip: Alt+drag moves the grid, Shift+click a beat line sets the downbeat.\nDownbeats are the brighter lines.").color(DIM).size(11.0));
        });
        });
        if sep { self.request_stems(i); }
        if remove { self.remove_sel(); }
    }
}

fn compat_col(c: u8) -> C { match c { 0 => C::from_rgb(90, 200, 110), 1 => C::from_rgb(230, 200, 60), _ => C::from_rgb(230, 80, 70) } }

#[derive(Clone)]
struct LibDrag(Vec<String>);

impl App {
    /// reference track for the key / BPM filters: the selected mix track, else the last one in the mix
    fn lib_ref(&self) -> Option<(String, String, f64)> {
        let i = self.sel.filter(|&i| i < self.mix.tracks.len()).or_else(|| self.mix.main_order().last().copied())?;
        let t = &self.mix.tracks[i]; Some((t.title.clone(), t.key.clone(), t.bpm))
    }
    fn lib_add(&mut self, paths: Vec<String>) {
        let v: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).filter(|p| p.exists()).collect();
        if v.is_empty() { self.status = "File not available (drive missing?)".into(); return; }
        self.status = format!("Adding {} track(s) from the library…", v.len()); self.load_files(v, None);
    }
    fn add_lib_folders(&mut self, v: Vec<PathBuf>) {
        let mut ok = vec![]; let mut bad = vec![];
        for p in v { let p = if let Some(r) = p.to_str().and_then(|s| s.strip_prefix("file://")) { PathBuf::from(r) } else { p };
            let p = if let Some(r) = p.to_str().and_then(|s| s.strip_prefix("~/")) { PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(r) } else { p };
            if p.is_dir() { self.lib.add_folder(&p); ok.push(p.display().to_string()); } else { bad.push(p.display().to_string()); } }
        self.tab = Tab::Library;
        self.status = match (ok.is_empty(), bad.is_empty()) {
            (false, true) => format!("Added folder: {} (scanning…)", ok.join(", ")),
            (false, false) => format!("Added {}; not a folder: {}", ok.join(", "), bad.join(", ")),
            _ => format!("Not a folder (or not available): {}", bad.join(", ")) };
    }
    fn pick_folder(&self) {
        let tx = self.tx.clone();
        let _ = tx.send(Msg::Status("Opening folder dialog…".into()));
        std::thread::spawn(move || match crate::dialogs::run(crate::dialogs::Req::Folders { title: "Add music folder".into() }) {
            Ok(Some(v)) => { let _ = tx.send(Msg::LibFolders(v)); }
            Ok(None) => { let _ = tx.send(Msg::Status("No folder chosen".into())); }
            Err(e) => { let _ = tx.send(Msg::Status(format!("Add folder: {e}"))); }
        });
    }
    fn library_tab(&mut self, ui: &mut egui::Ui) {
        let dim = |s: String| egui::RichText::new(s).color(DIM).size(11.0);
        let rf = self.lib_ref();
        let mut add: Option<Vec<String>> = None;
        ui.horizontal(|ui| {
            ui.label(dim("Folder path:".into()));
            let r = ui.add(egui::TextEdit::singleline(&mut self.lib_path).hint_text("type or paste a folder path, or drop a folder on the window").desired_width(360.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled(!self.lib_path.trim().is_empty(), egui::Button::new("Add path")).clicked() || enter) && !self.lib_path.trim().is_empty() {
                let p = PathBuf::from(self.lib_path.trim()); self.lib_path.clear(); self.add_lib_folders(vec![p]); }
        });
        ui.horizontal(|ui| {
            if ui.button("＋ Add folder…").on_hover_text("Add a music folder; it is scanned recursively and kept up to date at every start").clicked() { self.pick_folder(); }
            if ui.add_enabled(!self.lib.cfg.folders.is_empty(), egui::Button::new("⟳ Rescan now")).clicked() { self.lib.rescan(); }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.lib_q).hint_text("Search title, artist, album, genre…").desired_width(220.0));
            ui.checkbox(&mut self.lib_key, "Key-compatible").on_hover_text("Same key, ±1 or relative (Camelot) to the reference track");
            ui.checkbox(&mut self.lib_bpm, "BPM ±"); ui.add(egui::DragValue::new(&mut self.lib_pct).range(1.0..=30.0).speed(0.2).suffix(" %"));
            match &rf { Some((t, k, b)) => ui.label(dim(format!("relative to: {t} ({k}, {b:.1} BPM)"))), None => ui.label(dim("no reference track (add one to the mix)".into())) };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.lib_sel.len();
                if ui.add_enabled(n > 0, egui::Button::new(egui::RichText::new(format!("Add to mix ({n})")).color(if n > 0 { C::BLACK } else { DIM })).fill(if n > 0 { ACCENT } else { PANEL })).clicked() {
                    let mut v: Vec<String> = self.lib_sel.iter().cloned().collect(); v.sort(); add = Some(v); }
                let l = &self.lib;
                if l.scanning { ui.spinner(); ui.label(dim(if l.progress.1 > 0 { format!("Scanning: reading tags {}/{}", l.progress.0, l.progress.1) } else { "Scanning folders…".into() })); }
                else if l.analysing.1 > 0 { ui.add(egui::ProgressBar::new(l.analysing.0 as f32 / l.analysing.1 as f32).desired_width(120.0).text(format!("BPM/key {}/{}", l.analysing.0, l.analysing.1))); }
                else if let Some(r) = &l.report { ui.label(dim(format!("{} tracks · last scan: +{} new, {} changed, {} removed, {} unreadable{} in {:.1} s", r.files, r.added, r.updated, r.removed, r.errors + r.timed_out,
                    if r.timed_out > 0 { format!(" ({} timed out)", r.timed_out) } else { String::new() }, r.ms as f64 / 1000.0))); }
            });
        });
        // folders
        let mut remove: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            if self.lib.cfg.folders.is_empty() { ui.label(dim("No music folders yet. Click \"Add folder…\" to build your library.".into())); }
            for f in &self.lib.cfg.folders {
                let (txt, col) = match self.lib.states.get(f) { Some(crate::library::FolderState::Available(n)) => (format!("✔ {n}"), C::from_rgb(120, 200, 140)),
                    Some(crate::library::FolderState::Missing) => ("⚠ not available".to_string(), C::from_rgb(230, 170, 60)),
                    Some(crate::library::FolderState::NotResponding) => ("⚠ not responding".to_string(), C::from_rgb(230, 100, 80)), None => ("…".to_string(), DIM) };
                egui::Frame::none().fill(C::from_rgb(40, 42, 47)).rounding(4.0).inner_margin(egui::Margin::symmetric(6.0, 2.0)).show(ui, |ui| { ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(f).size(11.0)); ui.label(egui::RichText::new(txt).color(col).size(11.0))
                        .on_hover_text("Not available: the folder or drive is missing (its tracks are kept). Not responding: the drive did not answer in time."); 
                    if ui.small_button("✖").on_hover_text("Remove this folder from the library").clicked() { remove = Some(f.clone()); } }); });
            }
        });
        if let Some(f) = remove { self.lib.remove_folder(&f); self.lib_sel.clear(); }
        // filtered + sorted view, cached until something changes
        let key_ref = if self.lib_key { rf.as_ref().map(|r| r.1.clone()) } else { None };
        let bpm_ref = if self.lib_bpm { rf.as_ref().map(|r| (r.2, self.lib_pct)) } else { None };
        let vk = (self.lib.gen, self.lib_q.clone(), self.lib_sort, self.lib_asc, key_ref.clone(), bpm_ref.map(|b| (b.0.to_bits(), b.1.to_bits())));
        if self.lib_view_key.as_ref() != Some(&vk) {
            self.lib_view = crate::library::view(&self.lib.index.entries, &crate::library::Filter { query: &self.lib_q, key_ref: key_ref.as_deref(), bpm_ref }, self.lib_sort, self.lib_asc);
            self.lib_view_key = Some(vk);
        }
        use crate::library::SortCol;
        let cols = [(SortCol::Title, "Title"), (SortCol::Artist, "Artist"), (SortCol::Bpm, "BPM"), (SortCol::Key, "Key"), (SortCol::Duration, "Duration"), (SortCol::Genre, "Genre"), (SortCol::Folder, "Folder")];
        let mut clicked: Option<(usize, egui::Modifiers)> = None; let mut dbl: Option<usize> = None;
        let mods = ui.input(|i| i.modifiers);
        let avail = ui.available_height();
        egui_extras::TableBuilder::new(ui).striped(true).resizable(true).max_scroll_height(avail).min_scrolled_height(0.0)
            .column(egui_extras::Column::initial(260.0).at_least(80.0)).column(egui_extras::Column::initial(180.0)).column(egui_extras::Column::initial(56.0))
            .column(egui_extras::Column::initial(44.0)).column(egui_extras::Column::initial(62.0)).column(egui_extras::Column::initial(110.0)).column(egui_extras::Column::remainder())
            .header(18.0, |mut h| { for (c, n) in cols { h.col(|ui| { let arrow = if self.lib_sort == c { if self.lib_asc { " ⏶" } else { " ⏷" } } else { "" };
                if ui.add(egui::Label::new(egui::RichText::new(format!("{n}{arrow}")).size(10.5).color(if self.lib_sort == c { ACCENT } else { DIM })).sense(Sense::click())).clicked() {
                    if self.lib_sort == c { self.lib_asc = !self.lib_asc; } else { self.lib_sort = c; self.lib_asc = true; } } }); } })
            .body(|body| { let entries = &self.lib.index.entries; let view = &self.lib_view; let sel = &self.lib_sel; let states = &self.lib.states;
                body.rows(19.0, view.len(), |mut row| { let k = row.index(); let e = &entries[view[k]];
                    let on = sel.contains(&e.path);
                    let gone = !states.get(&e.folder).is_some_and(|s| matches!(s, crate::library::FolderState::Available(_)));
                    let col = if on { ACCENT } else if e.error.is_some() || gone { DIM } else { TXT };
                    let cells = [e.title.clone(), e.artist.clone(), e.bpm.map(|b| format!("{b:.1}")).unwrap_or_else(|| if e.analysis_error.is_some() { "–".into() } else { "…".into() }),
                        e.key.clone().unwrap_or_default(), fmt_t(e.duration), e.genre.clone(),
                        std::path::Path::new(&e.path).parent().and_then(|p| p.strip_prefix(&e.folder).ok()).map(|p| { let s = p.display().to_string(); if s.is_empty() { std::path::Path::new(&e.folder).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() } else { s } }).unwrap_or_default()];
                    for (ci, c) in cells.into_iter().enumerate() { row.col(|ui| {
                        let r = ui.add(egui::Label::new(egui::RichText::new(c).color(col).size(11.5)).truncate().sense(Sense::click_and_drag()));
                        let r = if ci == 0 { match (&e.error, gone) { (Some(err), _) => r.on_hover_text(format!("Unreadable: {err}")), (None, true) => r.on_hover_text("Folder not available"), _ => r.on_hover_text(&e.path) } } else { r };
                        if r.double_clicked() { dbl = Some(k); } else if r.clicked() { clicked = Some((k, mods)); }
                        if r.drag_started() { let paths: Vec<String> = if on { let mut v: Vec<String> = sel.iter().cloned().collect(); v.sort(); v } else { vec![e.path.clone()] };
                            r.dnd_set_drag_payload(LibDrag(paths)); }
                    }); }
                }); });
        if let Some(k) = dbl { add = Some(vec![self.lib.index.entries[self.lib_view[k]].path.clone()]); }
        if let Some((k, m)) = clicked {
            let p = self.lib.index.entries[self.lib_view[k]].path.clone();
            if m.shift { if let Some(a) = self.lib_anchor { let (x, y) = (a.min(k), a.max(k)); if !m.command { self.lib_sel.clear(); }
                    for j in x..=y.min(self.lib_view.len().saturating_sub(1)) { self.lib_sel.insert(self.lib.index.entries[self.lib_view[j]].path.clone()); } } }
            else if m.command { if !self.lib_sel.remove(&p) { self.lib_sel.insert(p); } self.lib_anchor = Some(k); }
            else { self.lib_sel.clear(); self.lib_sel.insert(p); self.lib_anchor = Some(k); }
        }
        if let Some(v) = add { self.lib_add(v); }
    }
}
