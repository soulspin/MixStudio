//! Scratch patterns: a turntable-style position curve for the audio under a Scratch block.
//!
//! Syntax (whitespace ignored, case-insensitive letters):
//! - `A` `B` `C` `D` `E`: record position targets within the current beat (A = beat start,
//!   E = one beat later). In a beat of only letters the keys are spread from the beat start to
//!   the beat end and the position glides between them (Easing: 0 = linear, 1 = smooth).
//!   A downward move plays in reverse. Beats containing `>` or `r` are split into equal slots.
//! - `>`: play normally (position follows time), `-`: hold the previous position (stopped record).
//! - `r`: release: the audio is muted (gating) and the record runs on normally.
//! - `.`: ends a beat. The pattern loops when the block is longer than the pattern.
//! Example: `AEA.` = baby scratch (forward and back in one beat), `>r>r.` = transformer.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key { Pos(f64), Normal, Hold, Release }

pub const PRESETS: [(&str, &str); 5] = [
    ("Baby scratch", "AEA.AEA."),
    ("Downer", "AC."),
    ("Transformer", ">r>r.>r>r."),
    ("Reverse", "EA."),
    ("Chirp", "AEr-."),
];

/// Parse a pattern into beats of keys. Errors name the first bad character.
pub fn parse(s: &str) -> Result<Vec<Vec<Key>>, String> {
    let mut beats = vec![]; let mut cur = vec![];
    for (i, c) in s.chars().enumerate() {
        match c.to_ascii_uppercase() {
            'A'..='E' => cur.push(Key::Pos((c.to_ascii_uppercase() as u8 - b'A') as f64 / 4.0)),
            '>' => cur.push(Key::Normal), '-' => cur.push(Key::Hold), 'R' => cur.push(Key::Release),
            '.' => { if cur.is_empty() { cur.push(Key::Normal); } beats.push(std::mem::take(&mut cur)); }
            c if c.is_whitespace() => {}
            c => return Err(format!("unknown character '{c}' at {}", i + 1)),
        }
    }
    if !cur.is_empty() { beats.push(cur); }
    if beats.is_empty() { return Err("empty pattern".into()); }
    Ok(beats)
}

fn ease(u: f64, e: f64) -> f64 { let s = u * u * (3.0 - 2.0 * u); u + (s - u) * e.clamp(0.0, 1.0) }

/// Position (in record beats from the block start) and gain at `t` beats after the block start.
/// A beat of only letters (and `-`) is a keyframe curve from beat start to beat end; a beat that
/// contains `>` or `r` is split into equal slots (for gating).
pub fn eval(p: &[Vec<Key>], t: f64, easing: f64) -> (f64, f32) {
    if p.is_empty() || t < 0.0 { return (t, 1.0); }
    let bi = t.floor(); let pi = bi as usize % p.len(); let beat = &p[pi]; let lt = t - bi; let n = beat.len();
    let val = |k: usize, at: f64| -> f64 { let mut k = k; loop { match beat[k] { Key::Pos(v) => return v, Key::Normal | Key::Release => return at, Key::Hold => { if k == 0 { return 0.0; } k -= 1; } } } };
    let keyframes = beat.iter().all(|k| matches!(k, Key::Pos(_) | Key::Hold));
    if keyframes {
        if n == 1 { return (bi + val(0, 0.0), 1.0); }
        let sf = lt * (n - 1) as f64; let j = (sf.floor() as usize).min(n - 2); let u = sf - j as f64;
        let (x, y) = (val(j, 0.0), val(j + 1, 0.0));
        return (bi + x + (y - x) * ease(u, easing), 1.0);
    }
    let sf = lt * n as f64; let j = (sf.floor() as usize).min(n - 1); let u = sf - j as f64; let st = |k: usize| k as f64 / n as f64;
    let pos = match beat[j] {
        Key::Normal | Key::Release => lt,
        Key::Hold => val(j, st(j)),
        Key::Pos(x) => { let y = if j + 1 < n { val(j + 1, st(j + 1)) } else { x }; x + (y - x) * ease(u, easing) }
    };
    let rel = |k: usize| matches!(beat[k], Key::Release);
    let prev_rel = if j > 0 { rel(j - 1) } else { matches!(p[(pi + p.len() - 1) % p.len()].last(), Some(Key::Release)) };
    let next_rel = if j + 1 < n { rel(j + 1) } else { matches!(p[(pi + 1) % p.len()].first(), Some(Key::Release)) };
    let edge = 0.012 * n as f64; // ~3 ms at 128 BPM, in slot units
    let g = if rel(j) { let i0 = if prev_rel { 0.0 } else { (1.0 - u / edge).max(0.0) }; let i1 = if next_rel { 0.0 } else { (1.0 - (1.0 - u) / edge).max(0.0) }; i0.max(i1) as f32 } else { 1.0 };
    (bi + pos, g)
}

/// Sample the curve for drawing: (t, position, gain) over `beats`.
pub fn curve(p: &[Vec<Key>], beats: f64, easing: f64, n: usize) -> Vec<(f64, f64, f32)> {
    (0..=n).map(|k| { let t = beats * k as f64 / n as f64; let (x, g) = eval(p, t.min(beats - 1e-9), easing); (t, x - t.floor(), g) }).collect()
}
