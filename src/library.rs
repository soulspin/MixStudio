//! Music library: watched folders, incremental recursive scan (index keyed by path + size + mtime),
//! tag reading, background BPM/key analysis with limited concurrency, and the filtered/sorted view.
//! Every file-system access that could stall on a failing or unplugged drive runs on a helper thread
//! with a timeout, so a bad disk can slow a scan down but never hangs it or the UI.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Extensions MixStudio's decoder (symphonia) can read.
pub const EXTS: &[&str] = &["mp3", "wav", "wave", "flac", "ogg", "oga", "m4a", "mp4", "aac", "alac", "aif", "aiff", "aifc"];
pub const INDEX_VERSION: u32 = 1;
const TAG_TIMEOUT: Duration = Duration::from_secs(15);
const WALK_TIMEOUT: Duration = Duration::from_secs(600);
const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(180);

pub fn is_audio(p: &Path) -> bool { p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str())) }

/// Preserve byte-backed drops as source files that saved projects can reopen.
pub fn store_drop(root: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let name=Path::new(name).file_name().filter(|n| !n.is_empty()).unwrap_or(std::ffi::OsStr::new("drop.wav"));
    std::fs::create_dir_all(root)?;
    let dir=root.join(crate::recovery::unique_id()); std::fs::create_dir(&dir)?;
    let p=dir.join(name);
    let result=(|| {
        let mut options=std::fs::OpenOptions::new(); options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file=options.open(&p)?; file.write_all(bytes)?; file.sync_all()
    })();
    if let Err(e)=result { let _=std::fs::remove_file(&p); let _=std::fs::remove_dir(&dir); return Err(e); }
    Ok(p)
}

fn home() -> PathBuf { std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")) }
/// ~/.config/mixstudio (Linux), ~/Library/Application Support/MixStudio (macOS), %APPDATA%\MixStudio (Windows).
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MIXSTUDIO_CONFIG_DIR") { return PathBuf::from(d); }
    if cfg!(windows) { return std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join("MixStudio"); }
    if cfg!(target_os = "macos") { return home().join("Library/Application Support/MixStudio"); }
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config")).join("mixstudio")
}
/// ~/.cache/mixstudio (Linux), ~/Library/Caches/MixStudio (macOS), %LOCALAPPDATA%\MixStudio (Windows).
pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MIXSTUDIO_CACHE_DIR") { return PathBuf::from(d); }
    if cfg!(windows) { return std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(home).join("MixStudio"); }
    if cfg!(target_os = "macos") { return home().join("Library/Caches/MixStudio"); }
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join("mixstudio")
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Config { pub folders: Vec<String>,
    /// library files of other DJ apps (read-only imports) and path prefix rewrites for them
    #[serde(default)] pub imports: Vec<String>, #[serde(default)] pub remaps: Vec<crate::import::Remap> }

/// `Entry::folder` of tracks shown from an imported library
pub const EXT_FOLDER: &str = "@import";

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Entry {
    pub path: String, pub folder: String, pub size: u64, pub mtime: u64,
    pub title: String, pub artist: String, pub album: String, pub genre: String, pub duration: f64,
    #[serde(default)] pub bpm: Option<f64>, #[serde(default)] pub key: Option<String>,
    /// file could not be read (tags); kept so it is not retried until it changes
    #[serde(default)] pub error: Option<String>,
    #[serde(default)] pub analysis_error: Option<String>,
    #[serde(default)] pub analysis_version: u32,
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Index { pub version: u32, pub entries: Vec<Entry> }

#[cfg(test)]
mod audit_tests {
    use super::*;
    #[test]
    fn dropped_sources_with_the_same_name_never_replace_each_other() {
        let dir=std::env::temp_dir().join(format!("mixstudio-drop-{}",crate::recovery::unique_id()));
        let first=store_drop(&dir,"song.wav",b"first").unwrap();
        let second=store_drop(&dir,"song.wav",b"second").unwrap();
        assert_ne!(first,second);
        assert_eq!(std::fs::read(first).unwrap(),b"first");
        let escaped=store_drop(&dir,"../outside.wav",b"safe").unwrap();
        assert!(escaped.strip_prefix(&dir).unwrap().components().all(|c|matches!(c,std::path::Component::Normal(_))));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn unicode_imported_keys_can_be_sorted_and_filtered() {
        let entries: Vec<_>=["F♯","♭","8A",""].iter().map(|k| Entry {key:Some(k.to_string()),..Default::default()}).collect();
        let filter=Filter {query:"",key_ref:Some("8A"),bpm_ref:None};
        assert_eq!(view(&entries,&filter,SortCol::Key,true),vec![2]);
        let filter=Filter {key_ref:None,..filter};
        assert_eq!(view(&entries,&filter,SortCol::Key,true).len(),4);
    }
    #[test]
    fn old_library_analysis_is_invalidated_without_losing_tags() {
        let dir=std::env::temp_dir().join(format!("mixstudio-index-{}",crate::recovery::unique_id()));
        let ix=Index {version:INDEX_VERSION,entries:vec![Entry {title:"Keep me".into(),bpm:Some(128.0),key:Some("8A".into()),..Default::default()}]};
        save_index(&dir,&ix).unwrap();
        let restored=load_index(&dir);
        assert_eq!(restored.entries[0].title,"Keep me");
        assert!(restored.entries[0].bpm.is_none(),"outdated BPM survives detector upgrade");
        let mut current=restored;
        current.entries[0].analysis_version=crate::analysis::ANALYSIS_VERSION;
        current.entries[0].bpm=Some(127.98);
        current.entries[0].analysis_error=Some("current decoder error".into());
        save_index(&dir,&current).unwrap();
        let restored=load_index(&dir);
        assert_eq!(restored.entries[0].bpm,Some(127.98));
        assert_eq!(restored.entries[0].analysis_error.as_deref(),Some("current decoder error"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn stale_analysis_results_cannot_overwrite_a_rescanned_library() {
        let dir=std::env::temp_dir().join(format!("mixstudio-stale-index-{}",crate::recovery::unique_id()));
        let mut l=Library::new(dir.clone(),dir,Arc::new(||{}));
        l.index.entries=vec![Entry {path:"same.wav".into(),bpm:Some(130.0),..Default::default()}]; l.reindex();
        let old=Arc::new(AtomicBool::new(true));
        l.tx.send(Event::Analysed(old.clone(),"same.wav".into(),Ok((120.0,"8A".into(),60.0)))).unwrap();
        let unrelated=Arc::new(AtomicBool::new(false));
        l.tx.send(Event::Analysed(unrelated.clone(),"same.wav".into(),Ok((110.0,"8A".into(),60.0)))).unwrap();
        l.analysing=(2,5);
        l.tx.send(Event::AnalysisDone(unrelated)).unwrap();
        l.poll();
        assert_eq!(l.index.entries[0].bpm,Some(130.0));
        assert_eq!(l.analysing,(2,5));
        drop(old);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FolderState { Available(usize), Missing, NotResponding }

#[derive(Clone, Debug, Default)]
pub struct ScanReport { pub files: usize, pub added: usize, pub updated: usize, pub unchanged: usize, pub removed: usize, pub errors: usize, pub timed_out: usize, pub missing: Vec<String>, pub ms: u128 }

fn read_json<T: for<'a> Deserialize<'a> + Default>(p: &Path) -> T { std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
fn write_json<T: Serialize>(p: &Path, v: &T) -> std::io::Result<()> {
    if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
    let tmp = p.with_extension("tmp"); std::fs::write(&tmp, serde_json::to_vec(v).unwrap_or_default())?; std::fs::rename(tmp, p)
}
pub fn load_config(dir: &Path) -> Config { read_json(&dir.join("library.json")) }
pub fn save_config(dir: &Path, c: &Config) -> std::io::Result<()> { write_json(&dir.join("library.json"), c) }
pub fn load_index(dir: &Path) -> Index {
    let mut i: Index = read_json(&dir.join("library-index.json"));
    if i.version != INDEX_VERSION { return Index { version: INDEX_VERSION, entries: vec![] }; }
    for e in &mut i.entries { if e.analysis_version != crate::analysis::ANALYSIS_VERSION {
        e.bpm=None; e.key=None; e.analysis_error=None;
    } }
    i
}
pub fn save_index(dir: &Path, i: &Index) -> std::io::Result<()> { write_json(&dir.join("library-index.json"), i) }

/// Run `f` on a helper thread; None if it takes longer than `t` (the thread is left to finish on its own) or panics.
pub fn with_timeout<T: Send + 'static>(t: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new().name("mixstudio-io".into()).spawn(move || { if let Ok(v) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) { let _ = tx.send(v); } }).ok()?;
    rx.recv_timeout(t).ok()
}

fn mtime(m: &std::fs::Metadata) -> u64 { m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0) }

fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, u64)>, errors: &mut usize, depth: usize, cancel: &AtomicBool) {
    if depth > 40 || cancel.load(Ordering::Relaxed) { return; }
    let rd = match std::fs::read_dir(dir) { Ok(r) => r, Err(_) => { *errors += 1; return; } };
    for e in rd {
        let Ok(e) = e else { *errors += 1; continue };
        let Ok(ft) = e.file_type() else { *errors += 1; continue };
        let p = e.path();
        if ft.is_dir() { if !e.file_name().to_string_lossy().starts_with('.') { walk(&p, out, errors, depth + 1, cancel); } }
        else if is_audio(&p) { match std::fs::metadata(&p) { Ok(m) if m.is_file() => out.push((p, m.len(), mtime(&m))), Ok(_) => {}, Err(_) => *errors += 1 } }
    }
}

pub struct Tags { pub title: String, pub artist: String, pub album: String, pub genre: String, pub duration: f64 }
/// Read tags and duration without decoding audio.
pub fn read_tags(path: &Path) -> Result<Tags, String> {
    use symphonia::core::{formats::FormatOptions, io::MediaSourceStream, meta::{MetadataOptions, StandardTagKey}, probe::Hint};
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(f), Default::default());
    let mut hint = Hint::new(); if let Some(e) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(e); }
    let mut pr = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| e.to_string())?;
    let mut t = Tags { title: String::new(), artist: String::new(), album: String::new(), genre: String::new(), duration: 0.0 };
    let mut read = |m: &symphonia::core::meta::MetadataRevision| { for tag in m.tags() { let v = tag.value.to_string(); match tag.std_key {
        Some(StandardTagKey::TrackTitle) => t.title = v, Some(StandardTagKey::Artist) => t.artist = v, Some(StandardTagKey::Album) => t.album = v, Some(StandardTagKey::Genre) => t.genre = v, _ => {} } } };
    if let Some(m) = pr.metadata.get() { if let Some(r) = m.current() { read(r); } }
    if let Some(r) = pr.format.metadata().current() { read(r); }
    let tr = pr.format.default_track().ok_or("no audio track")?;
    let cp = &tr.codec_params;
    if let (Some(n), Some(sr)) = (cp.n_frames, cp.sample_rate) { t.duration = n as f64 / sr as f64; }
    else if let (Some(n), Some(tb)) = (cp.n_frames, cp.time_base) { let tm = tb.calc_time(n); t.duration = tm.seconds as f64 + tm.frac; }
    if t.title.trim().is_empty() { t.title = path.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default(); }
    Ok(t)
}

/// Scan `folders` against the previous index. Unchanged files (same size and mtime) are reused without
/// reading them; new or changed files get their tags read (4 at a time, each with a timeout); files that
/// disappeared from an available folder are dropped. Entries of missing or unresponsive folders are kept.
pub fn scan(folders: &[String], old: &Index, cancel: &Arc<AtomicBool>, progress: &dyn Fn(usize, usize)) -> (Index, ScanReport, Vec<(String, FolderState)>) {
    let t0 = std::time::Instant::now(); let mut rep = ScanReport::default();
    let oldmap: HashMap<&str, &Entry> = old.entries.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut states = vec![]; let mut out: Vec<Entry> = vec![]; let mut seen: HashSet<String> = HashSet::new(); let mut todo: Vec<(PathBuf, u64, u64, String)> = vec![];
    for f in folders {
        let fp = PathBuf::from(f); let fc = fp.clone();
        let st = match with_timeout(Duration::from_secs(10), move || fc.is_dir()) {
            Some(true) => { let fc = fp.clone(); let cc = cancel.clone();
                match with_timeout(WALK_TIMEOUT, move || { let mut v = vec![]; let mut e = 0; walk(&fc, &mut v, &mut e, 0, &cc); (v, e) }) {
                    Some((files, errs)) => { rep.errors += errs; let n = files.len();
                        for (p, size, mt) in files { let ps = p.display().to_string(); if !seen.insert(ps.clone()) { continue; }
                            match oldmap.get(ps.as_str()) { Some(e) if e.size == size && e.mtime == mt => { let mut e = (*e).clone(); e.folder = f.clone(); out.push(e); rep.unchanged += 1; }
                                Some(_) => { rep.updated += 1; todo.push((p, size, mt, f.clone())); }
                                None => { rep.added += 1; todo.push((p, size, mt, f.clone())); } } }
                        FolderState::Available(n) }
                    None => FolderState::NotResponding } }
            Some(false) => FolderState::Missing,
            None => FolderState::NotResponding,
        };
        if !matches!(st, FolderState::Available(_)) {
            rep.missing.push(f.clone());
            for e in old.entries.iter().filter(|e| e.folder == *f) { if seen.insert(e.path.clone()) { out.push(e.clone()); } }
        }
        states.push((f.clone(), st));
    }
    // tags for new / changed files: 4 helper threads at a time, each with a deadline
    let total = todo.len(); let mut done = 0;
    for chunk in todo.chunks(4) {
        if cancel.load(Ordering::Relaxed) { break; }
        let rxs: Vec<_> = chunk.iter().map(|(p, ..)| { let (tx, rx) = crossbeam_channel::bounded(1); let p = p.clone();
            let _ = std::thread::Builder::new().name("mixstudio-tags".into()).spawn(move || { let r = std::panic::catch_unwind(|| read_tags(&p)).unwrap_or_else(|_| Err("decoder crashed".into())); let _ = tx.send(r); }); rx }).collect();
        let deadline = std::time::Instant::now() + TAG_TIMEOUT;
        for ((p, size, mt, f), rx) in chunk.iter().zip(rxs) {
            let r = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
            let ps = p.display().to_string();
            let mut e = Entry { path: ps.clone(), folder: f.clone(), size: *size, mtime: *mt, ..Default::default() };
            match r {
                Ok(Ok(t)) => { e.title = t.title; e.artist = t.artist; e.album = t.album; e.genre = t.genre; e.duration = t.duration; }
                Ok(Err(err)) => { rep.errors += 1; e.title = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(); e.error = Some(err); }
                Err(_) => { rep.timed_out += 1; e.title = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(); e.error = Some("read timed out (failing or slow drive?)".into()); }
            }
            out.push(e); done += 1;
        }
        progress(done, total);
    }
    let avail: HashSet<&str> = states.iter().filter(|s| matches!(s.1, FolderState::Available(_))).map(|s| s.0.as_str()).collect();
    rep.removed = old.entries.iter().filter(|e| avail.contains(e.folder.as_str()) && !seen.contains(&e.path)).count()
        + old.entries.iter().filter(|e| !folders.contains(&e.folder)).count();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    rep.files = out.len(); rep.ms = t0.elapsed().as_millis();
    (Index { version: INDEX_VERSION, entries: out }, rep, states)
}

/// BPM, key and duration through the shared analysis cache (decodes only when not cached yet).
pub fn analyse(path: &str) -> Result<(f64, String, f64), String> {
    let p = PathBuf::from(path);
    with_timeout(ANALYSIS_TIMEOUT, move || crate::analysis::load_analysis(&p).map(|a| (a.bpm, a.key, a.duration)).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Err("analysis timed out".into()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortCol { Title, Artist, Bpm, Key, Duration, Genre, Folder }

pub struct Filter<'a> { pub query: &'a str, pub key_ref: Option<&'a str>, pub bpm_ref: Option<(f64, f64)> }

fn camelot_ord(k: &Option<String>) -> (i32, u8) {
    k.as_deref().and_then(crate::automix::camelot).map(|(n,l)| (n,l as u8)).unwrap_or((99,0))
}

/// Indices of matching entries, sorted.
pub fn view(entries: &[Entry], f: &Filter, sort: SortCol, asc: bool) -> Vec<usize> {
    let words: Vec<String> = f.query.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut v: Vec<usize> = (0..entries.len()).filter(|&i| { let e = &entries[i];
        if !words.is_empty() { let hay = format!("{} {} {} {} {}", e.title, e.artist, e.album, e.genre, e.path).to_lowercase(); if !words.iter().all(|w| hay.contains(w)) { return false; } }
        if let Some(r) = f.key_ref { match &e.key { Some(k) if crate::automix::key_score(r, k).0 >= 0.84 => {}, _ => return false } }
        if let Some((b, pct)) = f.bpm_ref { match e.bpm { Some(x) if crate::automix::bpm_score(b, x).1.abs() <= pct => {}, _ => return false } }
        true }).collect();
    let fold = |s: &str| s.to_lowercase();
    v.sort_by(|&a, &b| { let (x, y) = (&entries[a], &entries[b]);
        let o = match sort {
            SortCol::Title => fold(&x.title).cmp(&fold(&y.title)), SortCol::Artist => fold(&x.artist).cmp(&fold(&y.artist)).then_with(|| fold(&x.title).cmp(&fold(&y.title))),
            SortCol::Bpm => x.bpm.unwrap_or(f64::MAX).partial_cmp(&y.bpm.unwrap_or(f64::MAX)).unwrap_or(std::cmp::Ordering::Equal),
            SortCol::Key => camelot_ord(&x.key).cmp(&camelot_ord(&y.key)), SortCol::Duration => x.duration.partial_cmp(&y.duration).unwrap_or(std::cmp::Ordering::Equal),
            SortCol::Genre => fold(&x.genre).cmp(&fold(&y.genre)), SortCol::Folder => x.path.cmp(&y.path) };
        if asc { o } else { o.reverse() } });
    v
}

pub enum Event { Imported(u64, Vec<Result<crate::import::ExtLib, String>>), Progress(usize, usize), Scanned(Index, ScanReport, Vec<(String, FolderState)>), Analysed(Arc<AtomicBool>, String, Result<(f64, String, f64), String>), AnalysisDone(Arc<AtomicBool>) }

/// Library state owned by the UI thread; scanning and analysis run on background threads.
pub struct Library {
    pub cfg: Config, pub index: Index, pub states: HashMap<String, FolderState>, pub report: Option<ScanReport>,
    pub scanning: bool, pub progress: (usize, usize), pub analysing: (usize, usize), pub gen: u64,
    cfg_dir: PathBuf, data_dir: PathBuf, tx: crossbeam_channel::Sender<Event>, rx: crossbeam_channel::Receiver<Event>,
    cancel: Arc<AtomicBool>, repaint: Arc<dyn Fn() + Send + Sync>, rescan_pending: bool, unsaved: usize, pos: HashMap<String, usize>,
    /// imported libraries (same order as cfg.imports) and path -> (import, track)
    pub ext: Vec<Result<crate::import::ExtLib, String>>, ext_map: HashMap<String, (usize, usize)>, imp_seq: u64,
}

impl Library {
    pub fn recovery_dir(&self)->PathBuf {self.cfg_dir.join("recovery")}

    pub fn new(cfg_dir: PathBuf, data_dir: PathBuf, repaint: Arc<dyn Fn() + Send + Sync>) -> Library {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut l = Library { cfg: load_config(&cfg_dir), index: load_index(&data_dir), states: HashMap::new(), report: None, scanning: false, progress: (0, 0), analysing: (0, 0), gen: 1,
            cfg_dir, data_dir, tx, rx, cancel: Arc::new(AtomicBool::new(false)), repaint, rescan_pending: false, unsaved: 0, pos: HashMap::new(), ext: vec![], ext_map: HashMap::new(), imp_seq: 0 };
        l.reindex(); l.reimport(); l
    }
    /// (Re)read every imported library on a helper thread (each with a timeout).
    pub fn reimport(&mut self) {
        self.imp_seq += 1; let seq = self.imp_seq;
        if self.cfg.imports.is_empty() { self.ext.clear(); self.ext_map.clear(); self.gen += 1; return; }
        let (files, maps, tx, rp) = (self.cfg.imports.clone(), self.cfg.remaps.clone(), self.tx.clone(), self.repaint.clone());
        std::thread::Builder::new().name("mixstudio-import".into()).spawn(move || {
            let v = files.iter().map(|f| { let (p, m) = (PathBuf::from(f), maps.clone()); with_timeout(Duration::from_secs(120), move || crate::import::load(&p, &m)).unwrap_or_else(|| Err("timed out (slow or failing drive?)".into())) }).collect();
            let _ = tx.send(Event::Imported(seq, v)); rp();
        }).ok();
    }
    pub fn add_import(&mut self, p: &Path) -> Result<(), String> {
        if !p.exists() { return Err(format!("{} not found", p.display())); }
        if crate::import::sniff(p).is_none() { return Err(format!("{}: not a Rekordbox / Traktor / Serato / Engine DJ / iTunes library file", p.display())); }
        let s = p.display().to_string(); if self.cfg.imports.contains(&s) { return Ok(()); }
        self.cfg.imports.push(s); let _ = save_config(&self.cfg_dir, &self.cfg); self.reimport(); Ok(())
    }
    pub fn remove_import(&mut self, i: usize) { if i < self.cfg.imports.len() { self.cfg.imports.remove(i); if i < self.ext.len() { let _ = self.ext.remove(i); } let _ = save_config(&self.cfg_dir, &self.cfg); self.reimport(); } }
    pub fn set_remaps(&mut self, r: Vec<crate::import::Remap>) { self.cfg.remaps = r; let _ = save_config(&self.cfg_dir, &self.cfg); self.reimport(); }
    /// the imported track for a (local) path, from the first import that has it
    pub fn ext_info(&self, path: &str) -> Option<(crate::import::Source, &crate::import::ExtTrack)> {
        let &(l, t) = self.ext_map.get(path)?; let lib = self.ext.get(l)?.as_ref().ok()?; Some((lib.source, lib.tracks.get(t)?)) }
    pub fn entry(&self, path: &str) -> Option<&Entry> { self.pos.get(path).and_then(|&i| self.index.entries.get(i)) }
    fn reindex(&mut self) { self.pos = self.index.entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect(); self.gen += 1; }
    pub fn rescan(&mut self) {
        if self.scanning { self.rescan_pending = true; return; }
        self.cancel.store(true, Ordering::Relaxed); self.cancel = Arc::new(AtomicBool::new(false)); self.analysing = (0, 0);
        self.scanning = true; self.progress = (0, 0);
        let (folders, old, tx, cancel, rp) = (self.cfg.folders.clone(), self.index.clone(), self.tx.clone(), self.cancel.clone(), self.repaint.clone());
        std::thread::Builder::new().name("mixstudio-scan".into()).spawn(move || {
            let txp = tx.clone(); let rpp = rp.clone();
            let r = scan(&folders, &old, &cancel, &move |d, t| { let _ = txp.send(Event::Progress(d, t)); rpp(); });
            let _ = tx.send(Event::Scanned(r.0, r.1, r.2)); rp();
        }).ok();
    }
    pub fn add_folder(&mut self, p: &Path) {
        let s = p.display().to_string(); if self.cfg.folders.contains(&s) { return; }
        self.cfg.folders.push(s); let _ = save_config(&self.cfg_dir, &self.cfg); self.rescan();
    }
    pub fn remove_folder(&mut self, f: &str) {
        self.cfg.folders.retain(|x| x != f); let _ = save_config(&self.cfg_dir, &self.cfg); self.states.remove(f);
        self.index.entries.retain(|e| e.folder != f); let _ = save_index(&self.data_dir, &self.index); self.reindex(); self.rescan();
    }
    fn start_analysis(&mut self) {
        let todo: Vec<String> = self.index.entries.iter().filter(|e| e.bpm.is_none() && e.error.is_none() && e.analysis_error.is_none() && self.states.get(&e.folder).is_some_and(|s| matches!(s, FolderState::Available(_)))).map(|e| e.path.clone()).collect();
        if todo.is_empty() { return; }
        self.analysing = (0, todo.len());
        // limited concurrency so the computer stays usable: 1 worker per 4 cores, at most 2
        let workers = std::thread::available_parallelism().map(|n| (n.get() / 4).clamp(1, 2)).unwrap_or(1);
        let (qtx, qrx) = crossbeam_channel::unbounded(); for p in todo { let _ = qtx.send(p); } drop(qtx);
        let left = Arc::new(std::sync::atomic::AtomicUsize::new(workers));
        for _ in 0..workers { let (qrx, tx, cancel, rp, left) = (qrx.clone(), self.tx.clone(), self.cancel.clone(), self.repaint.clone(), left.clone());
            std::thread::Builder::new().name("mixstudio-analyse".into()).spawn(move || {
                while let Ok(p) = qrx.recv() { if cancel.load(Ordering::Relaxed) { break; } let r = analyse(&p); let _ = tx.send(Event::Analysed(cancel.clone(), p, r)); rp(); }
                if left.fetch_sub(1, Ordering::Relaxed) == 1 { let _ = tx.send(Event::AnalysisDone(cancel.clone())); rp(); }
            }).ok(); }
    }
    /// Apply background results; call once per frame.
    pub fn poll(&mut self) {
        while let Ok(ev) = self.rx.try_recv() { match ev {
            Event::Imported(seq, v) => if seq == self.imp_seq {
                self.ext_map.clear();
                for (li, l) in v.iter().enumerate() { if let Ok(l) = l { for (ti, t) in l.tracks.iter().enumerate() { self.ext_map.entry(t.path.clone()).or_insert((li, ti)); } } }
                self.ext = v; self.gen += 1; },
            Event::Progress(d, t) => self.progress = (d, t),
            Event::Scanned(ix, rep, st) => { self.scanning = false;
                // keep folders the user removed while the scan ran out of the result
                let keep: HashSet<&String> = self.cfg.folders.iter().collect();
                self.index = Index { version: INDEX_VERSION, entries: ix.entries.into_iter().filter(|e| keep.contains(&e.folder)).collect() };
                self.states = st.into_iter().collect(); self.report = Some(rep); let _ = save_index(&self.data_dir, &self.index); self.reindex();
                if self.rescan_pending { self.rescan_pending = false; self.rescan(); } else { self.start_analysis(); } }
            Event::Analysed(cancel, p, r) => { if !Arc::ptr_eq(&cancel,&self.cancel) || cancel.load(Ordering::Relaxed) {continue;} self.analysing.0 += 1;
                if let Some(&i) = self.pos.get(&p) { let e = &mut self.index.entries[i];
                    e.analysis_version=crate::analysis::ANALYSIS_VERSION;
                    match r { Ok((b, k, d)) => { e.bpm = Some(b); e.key = Some(k); e.analysis_error=None; if e.duration <= 0.0 { e.duration = d; } } Err(err) => e.analysis_error = Some(err) } }
                self.gen += 1; self.unsaved += 1; if self.unsaved >= 25 { self.unsaved = 0; let _ = save_index(&self.data_dir, &self.index); } }
            Event::AnalysisDone(cancel) => { if !Arc::ptr_eq(&cancel,&self.cancel) || cancel.load(Ordering::Relaxed) {continue;} self.unsaved = 0; let _ = save_index(&self.data_dir, &self.index); self.analysing = (0, 0); }
        } }
    }
    #[allow(dead_code)]
    pub fn busy(&self) -> bool { self.scanning || self.analysing.1 > 0 }
}
