//! Phrase detection: per-bar energy / spectral features (3-band waveform peaks), novelty between
//! 4-bar windows at every 8-bar grid line, and heuristic labels from relative energy and trend.

pub const LABELS: [&str; 5] = ["Intro", "Build", "Drop", "Breakdown", "Outro"];
pub fn colour(k: u8) -> [u8; 3] { match k { 0 => [70, 140, 220], 1 => [230, 170, 40], 2 => [225, 60, 60], 3 => [140, 80, 210], _ => [60, 170, 150] } }

/// Mean [low, mid, high] peak level of every bar from the first downbeat (`off` seconds).
pub fn bar_features(p: &[[u8; 3]], peak_sr: u32, hop: usize, bpm: f64, off: f64) -> Vec<[f32; 3]> {
    let cps = peak_sr as f64 / hop as f64; let bar = 240.0 / bpm.max(1.0);
    let mut v = vec![]; let mut k = 0usize;
    loop {
        let s = ((off + k as f64 * bar) * cps).max(0.0) as usize; let e = ((off + (k + 1) as f64 * bar) * cps) as usize;
        if e > p.len() || e <= s { break; }
        let mut f = [0f32; 3]; for c in &p[s..e] { for j in 0..3 { f[j] += c[j] as f32; } }
        for x in f.iter_mut() { *x /= (e - s) as f32; } v.push(f); k += 1;
    }
    v
}

/// Novelty at bar k: summed band change between the 4 bars before and after (in standard deviations).
pub fn novelty(f: &[[f32; 3]]) -> Vec<f32> {
    let n = f.len(); let mut out = vec![0f32; n]; if n < 2 { return out; }
    let mut mean = [0f32; 3]; let mut sd = [0f32; 3];
    for j in 0..3 { mean[j] = f.iter().map(|x| x[j]).sum::<f32>() / n as f32; sd[j] = (f.iter().map(|x| (x[j] - mean[j]).powi(2)).sum::<f32>() / n as f32).sqrt().max(2.0); }
    for k in 1..n {
        let (a, b) = (k.saturating_sub(4), (k + 4).min(n));
        let mut s = 0f32;
        for j in 0..3 { let m1 = f[a..k].iter().map(|x| x[j]).sum::<f32>() / (k - a) as f32; let m2 = f[k..b].iter().map(|x| x[j]).sum::<f32>() / (b - k) as f32; s += (m2 - m1).abs() / sd[j]; }
        out[k] = s;
    }
    out
}

/// Phrases as (track beat from the first downbeat, label index into LABELS).
pub fn detect(f: &[[f32; 3]]) -> Vec<(f64, u8)> {
    let n = f.len(); if n < 8 { return vec![(0.0, 2)]; }
    let nov = novelty(f);
    let cands: Vec<usize> = (1..).map(|i| i * 8).take_while(|&k| k + 4 <= n).collect();
    let mx = cands.iter().map(|&k| nov[k]).fold(0f32, f32::max);
    let mut sorted: Vec<f32> = cands.iter().map(|&k| nov[k]).collect(); sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let thr = (4.0 * med).max(0.3).min(0.3 * mx).max(0.25);
    let mut bounds = vec![0usize];
    for &k in &cands { if nov[k] >= thr { bounds.push(k); } }
    // no phrase longer than 32 bars when a weaker 8-bar change exists inside it
    let mut i = 0;
    while i < bounds.len() {
        let s = bounds[i]; let e = bounds.get(i + 1).copied().unwrap_or(n);
        if e - s > 32 { if let Some(&k) = cands.iter().filter(|&&k| k > s + 4 && k + 4 < e && nov[k] >= 0.5 * thr).max_by(|&&a, &&b| nov[a].partial_cmp(&nov[b]).unwrap()) { bounds.insert(i + 1, k); continue; } }
        i += 1;
    }
    let segs: Vec<(usize, usize)> = bounds.iter().enumerate().map(|(i, &s)| (s, bounds.get(i + 1).copied().unwrap_or(n))).collect();
    let en = |a: usize, b: usize| f[a..b].iter().map(|x| x[0] + x[1] + x[2]).sum::<f32>() / (b - a).max(1) as f32;
    let es: Vec<f32> = segs.iter().map(|&(a, b)| en(a, b)).collect();
    let (lo, hi) = es.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
    let r: Vec<f32> = es.iter().map(|&x| if hi - lo < 1.0 { 1.0 } else { (x - lo) / (hi - lo) }).collect();
    let ns = segs.len();
    segs.iter().enumerate().map(|(i, &(a, b))| {
        let q = ((b - a) / 4).max(1); let rising = en(b - q, b) > en(a, a + q) * 1.15 + 1.0;
        let lab = if ns == 1 || r[i] >= 0.7 { 2 } else if i == 0 { 0 } else if i == ns - 1 && ns > 2 { 4 }
            else if rising || (r[i] >= 0.35 && r.get(i + 1).is_some_and(|&x| x >= 0.7) && (i == 0 || r[i - 1] < r[i])) { 1 } else { 3 };
        ((a * 4) as f64, lab)
    }).collect()
}
