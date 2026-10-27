//! Auto-order: scores every possible transition (Camelot key relation, tempo incl. half/double time,
//! optional energy curve) and searches for the best playlist order (multi-start greedy, simulated
//! annealing, then 2-opt / swap / relocate hill climbing). Locked positions never move.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Mode { Harmonic, Tempo, #[default] Balanced }
impl Mode { pub const ALL: [Mode; 3] = [Mode::Balanced, Mode::Harmonic, Mode::Tempo];
    pub fn name(self) -> &'static str { match self { Mode::Harmonic => "Harmonic first", Mode::Tempo => "Tempo first", Mode::Balanced => "Balanced" } }
    fn weights(self) -> (f64, f64) { match self { Mode::Harmonic => (0.75, 0.25), Mode::Tempo => (0.25, 0.75), Mode::Balanced => (0.5, 0.5) } } }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Curve { #[default] Off, Build, Peak, Flat }
impl Curve { pub const ALL: [Curve; 4] = [Curve::Off, Curve::Build, Curve::Peak, Curve::Flat];
    pub fn name(self) -> &'static str { match self { Curve::Off => "Ignore energy", Curve::Build => "Build up", Curve::Peak => "Peak in the middle", Curve::Flat => "Keep energy flat" } } }

#[derive(Clone, Debug)]
pub struct Item { pub key: String, pub bpm: f64, pub energy: f64 }

pub fn camelot(s: &str) -> Option<(i32, char)> {
    let s = s.trim(); let (last, l) = s.char_indices().next_back()?;
    let n: i32 = s[..last].parse().ok()?; let l = l.to_ascii_uppercase();
    if (1..=12).contains(&n) && (l == 'A' || l == 'B') { Some((n, l)) } else { None }
}

/// Key compatibility 0..1 and a short name for the move (a -> b).
pub fn key_score(a: &str, b: &str) -> (f64, &'static str) {
    let (Some((na, la)), Some((nb, lb))) = (camelot(a), camelot(b)) else { return (0.3, "unknown key") };
    let up = (nb - na).rem_euclid(12); let dist = up.min(12 - up);
    match (dist, la == lb, up) {
        (0, true, _) => (1.0, "same key"),
        (1, true, _) => (0.9, "±1"),
        (0, false, _) => (0.85, "relative"),
        (1, false, _) => (0.5, "diagonal"),
        (_, true, 2) => (0.45, "energy +2"),
        (_, true, 7) => (0.4, "energy +7"),
        (d, _, _) => ((0.3 - 0.06 * d as f64).max(0.0), "clash"),
    }
}

/// Tempo compatibility 0..1 and the tempo change in percent (b relative to a), allowing half/double time.
pub fn bpm_score(a: f64, b: f64) -> (f64, f64) {
    if a <= 0.0 || b <= 0.0 { return (0.0, 0.0); }
    let mut best = (f64::MAX, 1.0);
    for (m, pen) in [(1.0, 1.0), (2.0, 0.9), (0.5, 0.9)] { let pct = (b * m / a - 1.0) * 100.0; if pct.abs() / pen < best.0 { best = (pct.abs() / pen, m); } }
    let pct = (b * best.1 / a - 1.0) * 100.0;
    let pen = if best.1 == 1.0 { 1.0 } else { 0.9 };
    ((-(pct / 4.0).powi(2)).exp() * pen, pct)
}

/// Transition rating: (score 0..1, 0 = green / 1 = yellow / 2 = red, label like "8A➡9A ±1 · +1.6%").
pub fn rate(a: &Item, b: &Item) -> (f64, u8, String) {
    let (ks, kn) = key_score(&a.key, &b.key); let (bs, pct) = bpm_score(a.bpm, b.bpm);
    let s = 0.5 * ks + 0.5 * bs;
    let col = if ks >= 0.84 && bs >= 0.6 { 0 } else if ks >= 0.4 && bs >= 0.25 { 1 } else { 2 };
    (s, col, format!("{}➡{} {kn} · {pct:+.1}%", a.key, b.key))
}

pub struct Opts { pub mode: Mode, pub curve: Curve }

struct Ctx { n: usize, pair: Vec<f64>, en: Vec<f64>, curve: Curve, we: f64 }
impl Ctx {
    fn new(items: &[Item], o: &Opts) -> Ctx {
        let n = items.len(); let (wk, wb) = o.mode.weights();
        let mut pair = vec![0.0; n * n];
        for i in 0..n { for j in 0..n { if i != j { let ks = key_score(&items[i].key, &items[j].key).0; let bs = bpm_score(items[i].bpm, items[j].bpm).0; pair[i * n + j] = 1.0 - wk * ks - wb * bs; } } }
        // energy normalised by rank (robust to scale)
        let mut idx: Vec<usize> = (0..n).collect(); idx.sort_by(|&a, &b| items[a].energy.total_cmp(&items[b].energy));
        let mut en = vec![0.5; n]; if n > 1 { for (r, &i) in idx.iter().enumerate() { en[i] = r as f64 / (n - 1) as f64; } }
        Ctx { n, pair, en, curve: o.curve, we: 0.6 }
    }
    fn cost(&self, ord: &[usize]) -> f64 {
        let n = self.n; let mut c = 0.0;
        for w in ord.windows(2) { c += self.pair[w[0] * n + w[1]]; }
        if n > 1 { for (k, &i) in ord.iter().enumerate() { let p = k as f64 / (n - 1) as f64;
            c += self.we * match self.curve { Curve::Off => 0.0, Curve::Build => (self.en[i] - p).powi(2), Curve::Peak => (self.en[i] - (1.0 - (2.0 * p - 1.0).abs())).powi(2),
                Curve::Flat => if k > 0 { (self.en[i] - self.en[ord[k - 1]]).powi(2) } else { 0.0 } }; } }
        c
    }
}

/// Total cost of an order (lower is better), for reporting and tests.
pub fn order_cost(items: &[Item], ord: &[usize], o: &Opts) -> f64 { Ctx::new(items, o).cost(ord) }

/// Best order of `items` (indices). `locked[k]` keeps item k at position k.
pub fn optimize(items: &[Item], locked: &[bool], o: &Opts) -> Vec<usize> {
    let n = items.len(); if n < 2 { return (0..n).collect(); }
    let cx = Ctx::new(items, o);
    let free_pos: Vec<usize> = (0..n).filter(|&k| !locked[k]).collect();
    let free_items: Vec<usize> = free_pos.clone();
    if free_pos.len() < 2 { return (0..n).collect(); }
    // multi-start greedy: fill free positions left to right with the cheapest next item
    let greedy = |first: Option<usize>| -> Vec<usize> {
        let mut ord: Vec<usize> = (0..n).collect(); let mut left = free_items.clone();
        for &k in &free_pos {
            let pick = if k == free_pos[0] && first.is_some() { left.iter().position(|&x| Some(x) == first).unwrap() } else {
                let prev = if k > 0 { Some(ord[k - 1]) } else { None }; let next = if k + 1 < n && locked[k + 1] { Some(k + 1) } else { None };
                (0..left.len()).min_by(|&a, &b| { let c = |i: usize| prev.map(|p| cx.pair[p * n + i]).unwrap_or(0.0) + next.map(|q| cx.pair[i * n + q]).unwrap_or(0.0);
                    c(left[a]).total_cmp(&c(left[b])) }).unwrap() };
            ord[k] = left.remove(pick);
        }
        ord
    };
    let mut best = greedy(None); let mut bc = cx.cost(&best);
    if free_pos[0] == 0 { for &f in &free_items { let g = greedy(Some(f)); let c = cx.cost(&g); if c < bc { bc = c; best = g; } } }
    // simulated annealing over free positions (deterministic xorshift)
    let mut rng = 0x9E3779B97F4A7C15u64; let mut rnd = move |m: usize| { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; (rng % m as u64) as usize };
    let fp = free_pos.len(); let mut cur = best.clone(); let mut cc = bc;
    let iters = 4000 + 400 * n; let t0 = 0.3f64; let t1 = 0.001f64;
    let seg_free = |a: usize, b: usize| (a..=b).all(|k| !locked[k]);
    for it in 0..iters {
        let t = t0 * (t1 / t0).powf(it as f64 / iters as f64);
        let (a, b) = { let x = free_pos[rnd(fp)]; let y = free_pos[rnd(fp)]; (x.min(y), x.max(y)) }; if a == b { continue; }
        let mut cand = cur.clone();
        match rnd(3) { 0 => cand.swap(a, b), 1 if seg_free(a, b) => cand[a..=b].reverse(), 2 if seg_free(a, b) => { let v = cand.remove(a); cand.insert(b, v); }, _ => cand.swap(a, b) }
        let c = cx.cost(&cand);
        if c < cc || ((cc - c) / t).exp() > (rnd(1_000_000) as f64 / 1e6) { cur = cand; cc = c; if cc < bc { bc = cc; best = cur.clone(); } }
    }
    // final local search to a 2-opt / swap optimum
    loop {
        let mut improved = false;
        for ia in 0..fp { for ib in ia + 1..fp {
            let (a, b) = (free_pos[ia], free_pos[ib]);
            let mut c1 = best.clone(); c1.swap(a, b); let v = cx.cost(&c1); if v < bc - 1e-12 { best = c1; bc = v; improved = true; continue; }
            if seg_free(a, b) { let mut c2 = best.clone(); c2[a..=b].reverse(); let v = cx.cost(&c2); if v < bc - 1e-12 { best = c2; bc = v; improved = true; } }
        } }
        if !improved { break; }
    }
    best
}
