//! Decoding (symphonia), BPM/first-beat detection, key detection and 3-band waveform peaks.
use serde::{Deserialize, Serialize};
use std::path::Path;
use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};

pub struct Audio { pub sr: u32, pub l: Vec<f32>, pub r: Vec<f32> }
impl Audio { pub fn secs(&self) -> f64 { self.l.len() as f64 / self.sr as f64 } }

/// Samples per waveform peak column (at the track's own sample rate).
pub const PEAK_HOP: usize = 441;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Analysis {
    pub bpm: f64,
    pub offset: f64, // seconds to first downbeat
    pub key: String, // Camelot
    pub duration: f64,
    pub peaks: Vec<[u8; 3]>, // low, mid, high (0..255), one per PEAK_HOP samples
    pub peak_sr: u32,
    pub sections: Vec<(f64, u8)>, // (track beat, 0=DOWN 1=CHORUS 2=UP)
    pub title: String,
    pub artist: String,
}

pub fn decode(path: &Path) -> anyhow::Result<(Audio, String, String)> {
    let f = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(f), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(e); }
    let mut probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;
    let (mut title, mut artist) = (String::new(), String::new());
    let mut read_meta = |m: &symphonia::core::meta::MetadataRevision| {
        for t in m.tags() {
            match t.std_key { Some(symphonia::core::meta::StandardTagKey::TrackTitle) => title = t.value.to_string(),
                Some(symphonia::core::meta::StandardTagKey::Artist) => artist = t.value.to_string(), _ => {} }
        }
    };
    if let Some(m) = probed.metadata.get() { if let Some(r) = m.current() { read_meta(r); } }
    let mut fmt = probed.format;
    if let Some(r) = fmt.metadata().current() { read_meta(r); }
    let track = fmt.default_track().ok_or_else(|| anyhow::anyhow!("no audio track"))?.clone();
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44100);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    loop {
        let pkt = match fmt.next_packet() { Ok(p) => p, Err(_) => break };
        if pkt.track_id() != track.id { continue; }
        let buf = match dec.decode(&pkt) { Ok(b) => b, Err(symphonia::core::errors::Error::DecodeError(_)) => continue, Err(_) => break };
        let spec = *buf.spec();
        sr = spec.rate;
        let ch = spec.channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        for fr in sb.samples().chunks(ch) {
            l.push(fr[0]);
            r.push(if ch > 1 { fr[1] } else { fr[0] });
        }
    }
    if l.is_empty() { anyhow::bail!("could not decode audio"); }
    Ok((Audio { sr, l, r }, title, artist))
}

fn mono(a: &Audio) -> Vec<f32> { a.l.iter().zip(&a.r).map(|(x, y)| 0.5 * (x + y)).collect() }

/// Onset envelope: half-wave rectified difference of log energy (low-passed signal emphasised), hop 512.
fn onset_env(m: &[f32], sr: u32) -> (Vec<f32>, f64) {
    let hop = 512usize;
    let fps = sr as f64 / hop as f64;
    let a = (-2.0 * std::f32::consts::PI * 150.0 / sr as f32).exp();
    let mut lp = 0f32;
    let mut e = Vec::with_capacity(m.len() / hop + 1);
    for ch in m.chunks(hop) {
        let (mut s1, mut s2) = (0f32, 0f32);
        for &x in ch { lp = a * lp + (1.0 - a) * x; s1 += lp * lp; s2 += x * x; }
        e.push((1.0 + 200.0 * (2.0 * s1 + s2) / ch.len() as f32).ln());
    }
    let mut o = vec![0f32; e.len()];
    for i in 1..e.len() { o[i] = (e[i] - e[i - 1]).max(0.0); }
    // remove local mean
    let w = 8usize;
    let mut out = vec![0f32; o.len()];
    for i in 0..o.len() {
        let (s, t) = (i.saturating_sub(w), (i + w + 1).min(o.len()));
        let mean = o[s..t].iter().sum::<f32>() / (t - s) as f32;
        out[i] = (o[i] - mean).max(0.0);
    }
    // light gaussian smoothing so the comb search tolerates sub-frame phase
    let k = [0.25f32, 0.5, 1.0, 0.5, 0.25];
    let sm: Vec<f32> = (0..out.len()).map(|i| (0..5).map(|j| { let t = i as i64 + j as i64 - 2; if t < 0 || t as usize >= out.len() { 0.0 } else { out[t as usize] * k[j] } }).sum()).collect();
    (sm, fps)
}

/// Comb score of the onset envelope for a given period (frames) and phase (frames).
fn comb(o: &[f32], period: f64, phase: f64) -> f32 {
    let mut s = 0f32;
    let mut t = phase;
    let mut n = 0;
    while (t as usize) + 1 < o.len() {
        let i = t as usize; let f = (t - i as f64) as f32;
        s += o[i] * (1.0 - f) + o[i + 1] * f; n += 1;
        t += period;
    }
    if n == 0 { 0.0 } else { s / n as f32 }
}

/// Choose which of the 4 beat phases is the downbeat (bar 1).
/// Scores each phase by: low-band (kick/bass) energy on that beat, onset (kick attack) strength,
/// and energy jumps at 8-bar phrase boundaries aligned to that phase.
pub fn downbeat(m: &[f32], sr: u32, bpm: f64, first_beat: f64, onset: &[f32], fps: f64) -> f64 {
    let beat = 60.0 / bpm;
    let a = (-2.0 * std::f32::consts::PI * 120.0 / sr as f32).exp();
    let mut lp = 0f32;
    let win = (0.08 * sr as f64) as usize;
    let nbeats = ((m.len() as f64 / sr as f64 - first_beat) / beat) as usize;
    if nbeats < 16 { return first_beat; }
    // low-band energy in a short window after every beat
    let mut le = vec![0f32; nbeats];
    let mut idx = 0usize;
    for b in 0..nbeats {
        let st = ((first_beat + b as f64 * beat) * sr as f64) as usize;
        while idx < st && idx < m.len() { lp = a * lp + (1.0 - a) * m[idx]; idx += 1; }
        let mut e = 0f32;
        for i in st..(st + win).min(m.len()) { lp = a * lp + (1.0 - a) * m[i]; e += lp * lp; }
        idx = (st + win).min(m.len());
        le[b] = (1e-6 + e / win as f32).ln();
    }
    let on_at = |b: usize| -> f32 { let f = ((first_beat + b as f64 * beat) * fps) as usize; (f.saturating_sub(1)..(f + 2).min(onset.len())).map(|i| onset[i]).fold(0.0, f32::max) };
    let mean = le.iter().sum::<f32>() / nbeats as f32;
    let sd = (le.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / nbeats as f32).sqrt().max(1e-3);
    let omean = (0..nbeats).map(on_at).sum::<f32>() / nbeats as f32 + 1e-6;
    let mut best = (f32::MIN, 0usize);
    for k in 0..4 {
        let (mut el, mut eo, mut n) = (0f32, 0f32, 0f32);
        let mut b = k; while b < nbeats { el += (le[b] - mean) / sd; eo += on_at(b) / omean - 1.0; n += 1.0; b += 4; }
        // phrase boundaries: |energy(4 beats after) - energy(4 beats before)| every 32 beats, best of 8 sub-phases
        let mut phrase = 0f32;
        for mph in 0..8 { let mut s = 0f32; let mut c = 0f32; let mut b = k + 4 * mph + 32;
            while b + 4 < nbeats { let pre: f32 = le[b - 4..b].iter().sum(); let post: f32 = le[b..b + 4].iter().sum(); s += (post - pre).abs() / (4.0 * sd); c += 1.0; b += 32; }
            if c > 0.0 { phrase = phrase.max(s / c); } }
        let score = el / n + 0.7 * eo / n + 0.3 * phrase;
        if score > best.0 { best = (score, k); }
    }
    first_beat + best.1 as f64 * beat
}

pub fn detect_bpm(m: &[f32], sr: u32) -> (f64, f64) {
    let (o, fps) = onset_env(m, sr);
    if o.len() < 64 { return (128.0, 0.0); }
    // autocorrelation over 70..180 BPM with mild tempo prior around 125
    let mut best = (0f64, 128.0f64);
    let mut bpm = 70.0;
    while bpm <= 180.0 {
        let lag = fps * 60.0 / bpm;
        let li = lag as usize; let f = (lag - li as f64) as f32;
        let mut ac = 0f32;
        for i in 0..o.len().saturating_sub(li + 1) { ac += o[i] * (o[i + li] * (1.0 - f) + o[i + li + 1] * f); }
        // also reward the double period (bar coherence)
        let w = (-0.5 * ((bpm / 125.0).log2() / 0.6).powi(2)).exp();
        let score = ac as f64 * (0.6 + 0.4 * w);
        if score > best.0 { best = (score, bpm); }
        bpm += 0.25;
    }
    // fine search with comb filter over phases
    let mut fine = (0f32, best.1, 0.0f64);
    let mut b = best.1 - 1.5;
    while b <= best.1 + 1.5 {
        let p = fps * 60.0 / b;
        let steps = 48;
        for k in 0..steps {
            let ph = p * k as f64 / steps as f64;
            let s = comb(&o, p, ph);
            if s > fine.0 { fine = (s, b, ph); }
        }
        b += 0.01;
    }
    let mut bpm = fine.1;
    if (bpm - bpm.round()).abs() < 0.08 { bpm = bpm.round(); }
    let period = fps * 60.0 / bpm;
    // refine phase at chosen bpm
    let mut ph = (0f32, fine.2);
    for k in 0..64 { let p = period * k as f64 / 64.0; let s = comb(&o, period, p); if s > ph.0 { ph = (s, p); } }
    let beat_ph = ph.1; // frames, phase of beats
    let off_beat = (beat_ph / fps).max(0.0);
    let off = downbeat(m, sr, bpm, off_beat, &o, fps);
    (bpm, off)
}

const MAJ: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MIN: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

/// pitch class (0=C) + minor -> Camelot
pub fn camelot(pc: usize, minor: bool) -> String {
    let maj = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
    let n = if minor { maj[(pc + 3) % 12] } else { maj[pc] };
    format!("{}{}", n, if minor { "A" } else { "B" })
}

pub fn detect_key(m: &[f32], sr: u32) -> String {
    // downsample by 4 (box average) then FFT chromagram
    let d = 4usize;
    let x: Vec<f32> = m.chunks(d).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let fs = sr as f32 / d as f32;
    let n = 8192usize;
    if x.len() < n { return "?".into(); }
    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect();
    let mut chroma = [0f32; 12];
    let mut buf = vec![rustfft::num_complex::Complex32::new(0.0, 0.0); n];
    let mut pos = 0;
    let bin_pc: Vec<Option<usize>> = (0..n / 2).map(|k| {
        let f = k as f32 * fs / n as f32;
        if !(55.0..2000.0).contains(&f) { return None; }
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        Some((midi.round() as i32).rem_euclid(12) as usize)
    }).collect();
    while pos + n <= x.len() {
        for i in 0..n { buf[i] = rustfft::num_complex::Complex32::new(x[pos + i] * win[i], 0.0); }
        fft.process(&mut buf);
        let mut fr = [0f32; 12];
        for k in 0..n / 2 { if let Some(pc) = bin_pc[k] { fr[pc] += buf[k].norm().sqrt(); } }
        let s: f32 = fr.iter().sum::<f32>() + 1e-9;
        for i in 0..12 { chroma[i] += fr[i] / s; }
        pos += n;
    }
    let corr = |p: &[f32; 12], root: usize| -> f32 {
        let mc = chroma.iter().sum::<f32>() / 12.0; let mp = p.iter().sum::<f32>() / 12.0;
        let (mut a, mut b, mut c) = (0f32, 0f32, 0f32);
        for i in 0..12 { let x = chroma[(i + root) % 12] - mc; let y = p[i] - mp; a += x * y; b += x * x; c += y * y; }
        a / (b * c).sqrt().max(1e-9)
    };
    let mut best = (-2f32, 0usize, false);
    for r in 0..12 { for (p, mi) in [(&MAJ, false), (&MIN, true)] { let c = corr(p, r); if c > best.0 { best = (c, r, mi); } } }
    camelot(best.1, best.2)
}

pub fn peaks(a: &Audio) -> Vec<[u8; 3]> {
    let sr = a.sr as f32;
    let al = (-2.0 * std::f32::consts::PI * 200.0 / sr).exp();
    let ah = (-2.0 * std::f32::consts::PI * 2500.0 / sr).exp();
    let (mut lp, mut lp2) = (0f32, 0f32);
    let mut out = Vec::with_capacity(a.l.len() / PEAK_HOP + 1);
    let (mut pl, mut pm, mut ph) = (0f32, 0f32, 0f32);
    for (i, (x, y)) in a.l.iter().zip(&a.r).enumerate() {
        let s = 0.5 * (x + y);
        lp = al * lp + (1.0 - al) * s;
        lp2 = ah * lp2 + (1.0 - ah) * s;
        let low = lp; let high = s - lp2; let mid = lp2 - lp;
        pl = pl.max(low.abs()); pm = pm.max(mid.abs()); ph = ph.max(high.abs());
        if i % PEAK_HOP == PEAK_HOP - 1 {
            let q = |v: f32| (v.min(1.0).sqrt() * 255.0) as u8;
            out.push([q(pl), q(pm * 1.3), q(ph * 2.0)]);
            pl = 0.0; pm = 0.0; ph = 0.0;
        }
    }
    out
}

/// Very rough sections per 16 bars from low-band energy.
fn sections(p: &[[u8; 3]], peak_sr: u32, bpm: f64, off: f64) -> Vec<(f64, u8)> {
    let cols_per_beat = 60.0 / bpm * peak_sr as f64 / PEAK_HOP as f64;
    let seg = 32.0; // beats
    let mut res = Vec::new();
    let total_beats = p.len() as f64 / cols_per_beat - off * bpm / 60.0;
    let mut e = Vec::new();
    let mut b = 0.0;
    while b < total_beats {
        let s = ((b + off * bpm / 60.0) * cols_per_beat) as usize; let t = (((b + seg) + off * bpm / 60.0) * cols_per_beat) as usize;
        let t = t.min(p.len()); if s >= t { break; }
        e.push((b, p[s..t].iter().map(|v| v[0] as f32).sum::<f32>() / (t - s) as f32));
        b += seg;
    }
    let mx = e.iter().map(|x| x.1).fold(1f32, f32::max);
    for (i, &(b, v)) in e.iter().enumerate() {
        let next = e.get(i + 1).map(|x| x.1).unwrap_or(v);
        let k = if v > 0.75 * mx { 1 } else if next > v * 1.25 { 2 } else { 0 };
        res.push((b, k));
    }
    res
}

fn cache_path(path: &Path) -> Option<std::path::PathBuf> {
    let md = std::fs::metadata(path).ok()?;
    let mut h: u64 = 1469598103934665603;
    for b in path.to_string_lossy().bytes().chain(md.len().to_le_bytes()) { h = (h ^ b as u64).wrapping_mul(1099511628211); }
    if let Ok(m) = md.modified() { if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) { h ^= d.as_secs(); } }
    let home = std::env::var("HOME").ok()?;
    Some(Path::new(&home).join(".cache/mixstudio").join(format!("{h:016x}.json")))
}

/// Decode + analyse, using the on-disk analysis/waveform cache when present.
pub fn load(path: &Path) -> anyhow::Result<(Audio, Analysis)> {
    let (audio, title, artist) = decode(path)?;
    let cp = cache_path(path);
    if let Some(c) = &cp { if let Ok(s) = std::fs::read(c) { if let Ok(a) = serde_json::from_slice::<Analysis>(&s) { return Ok((audio, a)); } } }
    let a = analyse(&audio, title, artist, path);
    if let Some(c) = cp { let _ = std::fs::create_dir_all(c.parent().unwrap()); let _ = std::fs::write(c, serde_json::to_vec(&a).unwrap_or_default()); }
    Ok((audio, a))
}

pub fn analyse(audio: &Audio, title: String, artist: String, path: &Path) -> Analysis {
    let m = mono(audio);
    let (bpm, offset) = detect_bpm(&m, audio.sr);
    let key = detect_key(&m, audio.sr);
    let peaks = peaks(audio);
    let sections = sections(&peaks, audio.sr, bpm, offset);
    let title = if title.is_empty() { path.file_stem().map(|s| s.to_string_lossy().replace("_", " ")).unwrap_or_default() } else { title };
    Analysis { bpm, offset, key, duration: audio.secs(), peaks, peak_sr: audio.sr, sections, title, artist }
}
