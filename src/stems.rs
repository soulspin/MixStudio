//! AI stem separation with HT-Demucs (MIT, Meta/facebookresearch) exported to ONNX by StemSplit
//! (huggingface.co/StemSplitio/htdemucs-onnx, MIT), run with ONNX Runtime through the `ort` crate.
//! ONNX Runtime is loaded dynamically at runtime (CUDA execution provider when available, CPU otherwise).
//! The model is downloaded on first use and verified by SHA-256.
use crate::analysis::{Audio, PEAK_HOP};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const MODEL_FILE: &str = "htdemucs_fp16weights.onnx";
pub const MODEL_URL: &str = "https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/d54ed9eb60e258ea82131c6ee14578628816456a/htdemucs_fp16weights.onnx";
pub const MODEL_SHA256: &str = "d05c269d0178d2a72ad484b10b11dd370193fc923201c3b27a99f848745db70a";
pub const MODEL_SIZE: u64 = 165_612_636;
const SR: u32 = 44100;
const SEG: usize = 343_980; // 7.8 s, fixed model input length

/// Separated stems at the track's own sample rate and length. Drums, bass and vocals are stored
/// as i16; melody ("other") is derived as mix - (drums + bass + vocals), so the four always sum to the mix.
pub struct Stems { pub d: [Vec<i16>; 2], pub b: [Vec<i16>; 2], pub v: [Vec<i16>; 2], pub peaks: Vec<[u8; 4]> }
impl Stems {
    #[inline] pub fn get(&self, k: usize, ch: usize, i: usize) -> f32 {
        let s = match k { 0 => &self.d, 1 => &self.b, _ => &self.v }; s[ch].get(i).map(|x| *x as f32 / 32767.0).unwrap_or(0.0)
    }
    /// sample of the weighted stem sum: g = gains for [drums, bass, melody, vocals]
    #[inline] pub fn mixed(&self, a: &Audio, ch: usize, i: usize, g: &[f32; 4]) -> f32 {
        let m = if ch == 0 { a.l[i] } else { a.r[i] };
        g[2] * m + (g[0] - g[2]) * self.get(0, ch, i) + (g[1] - g[2]) * self.get(1, ch, i) + (g[3] - g[2]) * self.get(2, ch, i)
    }
}

pub fn models_dir() -> PathBuf { let h = std::env::var("HOME").unwrap_or_else(|_| ".".into()); Path::new(&h).join(".local/share/mixstudio/models") }
pub fn stems_dir() -> PathBuf { let h = std::env::var("HOME").unwrap_or_else(|_| ".".into()); Path::new(&h).join(".cache/mixstudio/stems") }
pub fn model_path() -> PathBuf { models_dir().join(MODEL_FILE) }

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?; let mut h = Sha256::new(); let mut buf = vec![0u8; 1 << 20];
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download (if needed) and verify the model. `progress` receives 0..1000.
pub fn ensure_model(progress: &AtomicU32) -> anyhow::Result<PathBuf> {
    let p = model_path();
    if p.exists() && std::fs::metadata(&p)?.len() == MODEL_SIZE { return Ok(p); }
    std::fs::create_dir_all(models_dir())?;
    let tmp = p.with_extension("part");
    let resp = ureq::get(MODEL_URL).call().map_err(|e| anyhow::anyhow!("model download failed: {e}"))?;
    let total: u64 = resp.header("content-length").and_then(|s| s.parse().ok()).unwrap_or(MODEL_SIZE);
    let mut r = resp.into_reader(); let mut f = std::fs::File::create(&tmp)?; let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16]; let mut got = 0u64;
    loop { let n = r.read(&mut buf)?; if n == 0 { break; } f.write_all(&buf[..n])?; h.update(&buf[..n]); got += n as u64;
        progress.store((got * 1000 / total.max(1)).min(1000) as u32, Ordering::Relaxed); }
    f.flush()?; drop(f);
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if hex != MODEL_SHA256 { let _ = std::fs::remove_file(&tmp); anyhow::bail!("model checksum mismatch (got {hex})"); }
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}
pub fn verify_model() -> bool { sha256_file(&model_path()).map(|h| h == MODEL_SHA256).unwrap_or(false) }

static ENV: OnceLock<Result<(ort::environment::Environment, String), String>> = OnceLock::new();
static SESSION: Mutex<Option<(ort::session::Session, String)>> = Mutex::new(None);

/// Candidate ONNX Runtime libraries: $ORT_DYLIB_PATH, the system package, or a copy next to the binary.
fn find_ort() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = vec![];
    if let Some(p) = std::env::var_os("ORT_DYLIB_PATH") { c.push(p.into()); }
    for p in ["/usr/lib/libonnxruntime.so", "/usr/lib/libonnxruntime.so.1", "/usr/lib64/libonnxruntime.so", "/usr/local/lib/libonnxruntime.so", "/usr/lib/x86_64-linux-gnu/libonnxruntime.so"] { c.push(p.into()); }
    if let Ok(exe) = std::env::current_exe() { if let Some(d) = exe.parent() { c.push(d.join("lib/libonnxruntime.so")); c.push(d.join("libonnxruntime.so")); } }
    c.into_iter().find(|p| p.exists())
}
fn env() -> Result<(), String> {
    ENV.get_or_init(|| {
        let p = find_ort().ok_or("ONNX Runtime not found. Install it (Arch/CachyOS: sudo pacman -S onnxruntime-cuda, or onnxruntime-cpu) or set ORT_DYLIB_PATH.")?;
        let b = ort::init_from(&p).map_err(|e| format!("could not load {}: {e}", p.display()))?;
        b.with_name("mixstudio").build().map(|e| (e, p.display().to_string())).map_err(|e| e.to_string())
    }).as_ref().map(|_| ()).map_err(|e| e.clone())
}

/// Load the session once. Returns the execution provider in use ("CUDA" or "CPU").
pub fn session_info() -> Result<String, String> {
    env()?;
    let envr = &ENV.get().unwrap().as_ref().unwrap().0;
    let mut s = SESSION.lock().unwrap();
    if let Some((_, ep)) = s.as_ref() { return Ok(ep.clone()); }
    let path = model_path(); if !path.exists() { return Err("model not downloaded".into()); }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).max(1);
    let mk = |cuda: bool| -> Result<(ort::session::Session, String), String> {
        let mut b = ort::session::Session::builder(envr).map_err(|e| e.to_string())?;
        let mut ep = "CPU".to_string();
        if cuda && std::env::var_os("MIXSTUDIO_CPU").is_none() {
            use ort::ep::ExecutionProvider;
            if ort::ep::CUDA::default().register(&mut b).is_ok() { ep = "CUDA".into(); }
        }
        let mut b = b.with_intra_threads(threads).map_err(|e| e.to_string())?;
        b.commit_from_file(&path).map(|s| (s, ep)).map_err(|e| e.to_string())
    };
    let r = mk(true).or_else(|_| mk(false))?;
    let ep = r.1.clone(); *s = Some(r); Ok(ep)
}

fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to { return x.to_vec(); }
    let n = (x.len() as f64 * to as f64 / from as f64) as usize; let r = from as f64 / to as f64;
    (0..n).map(|i| { let p = i as f64 * r; let k = p as usize; let f = (p - k as f64) as f32; let a = x[k.min(x.len() - 1)]; let b = x[(k + 1).min(x.len() - 1)]; a + (b - a) * f }).collect()
}

/// Run the model. Returns raw stems [4][2] at the track's sample rate (drums, bass, other, vocals).
pub fn separate_raw(a: &Audio, progress: &AtomicU32) -> Result<Vec<[Vec<f32>; 2]>, String> {
    session_info()?;
    let l = resample(&a.l, a.sr, SR); let r = resample(&a.r, a.sr, SR);
    let total = l.len(); let overlap = SEG / 4; let stride = SEG - overlap;
    let n = ((total + stride - 1) / stride).max(1);
    let mut win = vec![1f32; SEG]; for i in 0..overlap { let f = i as f32 / (overlap - 1) as f32; win[i] = f; win[SEG - 1 - i] = f; }
    let mut out: Vec<[Vec<f32>; 2]> = (0..4).map(|_| [vec![0f32; total], vec![0f32; total]]).collect();
    let mut wsum = vec![0f32; total];
    let mut guard = SESSION.lock().unwrap(); let sess = &mut guard.as_mut().unwrap().0;
    for c in 0..n {
        let st = c * stride; let en = (st + SEG).min(total); let len = en - st;
        let mut x = vec![0f32; 2 * SEG]; x[..len].copy_from_slice(&l[st..en]); x[SEG..SEG + len].copy_from_slice(&r[st..en]);
        let t = ort::value::Tensor::from_array(([1usize, 2, SEG], x.into_boxed_slice())).map_err(|e| e.to_string())?;
        let o = sess.run(ort::inputs!["mix" => t]).map_err(|e| e.to_string())?;
        let (_, y) = o["stems"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        for k in 0..4 { for ch in 0..2 { let base = (k * 2 + ch) * SEG; for i in 0..len { out[k][ch][st + i] += y[base + i] * win[i]; } } }
        for i in 0..len { wsum[st + i] += win[i]; }
        progress.store(((c + 1) * 1000 / n) as u32, Ordering::Relaxed);
    }
    for k in 0..4 { for ch in 0..2 { for i in 0..total { out[k][ch][i] /= wsum[i].max(1e-8); } } }
    if a.sr != SR { for k in 0..4 { for ch in 0..2 { let mut v = resample(&out[k][ch], SR, a.sr); v.resize(a.l.len(), 0.0); out[k][ch] = v; } } }
    Ok(out)
}

fn q(v: &[f32]) -> Vec<i16> { v.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect() }
pub fn from_raw(a: &Audio, raw: &[[Vec<f32>; 2]]) -> Stems {
    let s = Stems { d: [q(&raw[0][0]), q(&raw[0][1])], b: [q(&raw[1][0]), q(&raw[1][1])], v: [q(&raw[3][0]), q(&raw[3][1])], peaks: vec![] };
    with_peaks(a, s)
}
fn with_peaks(a: &Audio, mut s: Stems) -> Stems {
    let n = a.l.len() / PEAK_HOP; let mut p = vec![[0u8; 4]; n];
    for c in 0..n { let mut m = [0f32; 4];
        for i in c * PEAK_HOP..(c + 1) * PEAK_HOP { let mut parts = [0f32; 4]; for ch in 0..2 { let d = s.get(0, ch, i); let b = s.get(1, ch, i); let v = s.get(2, ch, i);
            let mx = if ch == 0 { a.l[i] } else { a.r[i] }; parts[0] += d.abs(); parts[1] += b.abs(); parts[3] += v.abs(); parts[2] += (mx - d - b - v).abs(); }
            for k in 0..4 { m[k] = m[k].max(parts[k] * 0.5); } }
        p[c] = [0, 1, 2, 3].map(|k| (m[k].min(1.0).sqrt() * 255.0) as u8); }
    s.peaks = p; s
}

pub fn cache_file(key: &str) -> PathBuf { stems_dir().join(format!("{key}.stems")) }
pub fn save(key: &str, s: &Stems) -> std::io::Result<()> {
    std::fs::create_dir_all(stems_dir())?; let mut v = Vec::with_capacity(16 + s.d[0].len() * 12);
    v.extend(b"MSST1\0\0\0"); v.extend((s.d[0].len() as u64).to_le_bytes());
    for ch in s.d.iter().chain(s.b.iter()).chain(s.v.iter()) { for x in ch { v.extend(x.to_le_bytes()); } }
    let tmp = cache_file(key).with_extension("part"); std::fs::write(&tmp, v)?; std::fs::rename(tmp, cache_file(key))
}
pub fn load(key: &str, a: &Audio) -> Option<Stems> {
    let b = std::fs::read(cache_file(key)).ok()?;
    if b.len() < 16 || &b[..5] != b"MSST1" { return None; }
    let n = u64::from_le_bytes(b[8..16].try_into().ok()?) as usize;
    if n != a.l.len() || b.len() != 16 + n * 12 { return None; }
    let rd = |k: usize| -> Vec<i16> { let o = 16 + k * n * 2; b[o..o + n * 2].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect() };
    Some(with_peaks(a, Stems { d: [rd(0), rd(1)], b: [rd(2), rd(3)], v: [rd(4), rd(5)], peaks: vec![] }))
}

/// Full pipeline for a track: cached stems, or model download + separation + cache.
pub fn get_or_separate(key: &str, a: &Audio, dl: &AtomicU32, prog: &AtomicU32) -> Result<Arc<Stems>, String> {
    if let Some(s) = load(key, a) { prog.store(1000, Ordering::Relaxed); return Ok(Arc::new(s)); }
    ensure_model(dl).map_err(|e| e.to_string())?;
    let raw = separate_raw(a, prog)?;
    let s = from_raw(a, &raw);
    let _ = save(key, &s);
    Ok(Arc::new(s))
}

/// Render the weighted stem sum `g` of source seconds `a0..a1` as a new clip.
pub fn render_clip(a: &Audio, s: &Stems, g: &[f32; 4], a0: f64, a1: f64) -> Audio {
    let n = a.l.len(); let i0 = ((a0.max(0.0) * a.sr as f64) as usize).min(n); let i1 = ((a1.max(0.0) * a.sr as f64) as usize).min(n).max(i0);
    Audio { sr: a.sr, l: (i0..i1).map(|i| s.mixed(a, 0, i, g)).collect(), r: (i0..i1).map(|i| s.mixed(a, 1, i, g)).collect() }
}
