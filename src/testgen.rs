//! Synthesised test tracks (kick + bass + chord pad) for smoke tests and the demo screenshot.
use std::f32::consts::PI;
pub fn synth(bpm: f32, root_midi: i32, minor: bool, secs: f32, lead_in: f32, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let n = (secs * sr as f32) as usize; let mut l = vec![0f32; n]; let mut r = vec![0f32; n];
    let beat = 60.0 / bpm; let hz = |m: i32| 440.0 * 2f32.powf((m - 69) as f32 / 12.0);
    let third = if minor { 3 } else { 4 };
    let chord = [root_midi + 12, root_midi + 12 + third, root_midi + 19, root_midi + 24];
    let mut seed = 1u32;
    for i in 0..n {
        let t = i as f32 / sr as f32 - lead_in;
        let mut s = 0.0;
        if t >= 0.0 {
            let bar = (t / (beat * 4.0)) as i32;
            let section = (bar / 16) % 3; // 0 full, 1 breakdown, 2 full
            let tb = t % beat;
            let acc = if ((t / beat) as i32) % 4 == 0 { 1.0 } else { 0.55 };
            if section != 1 { let f = 50.0 + 120.0 * (-tb * 30.0).exp(); s += 0.8 * acc * (2.0 * PI * f * tb).sin() * (-tb * 7.0).exp(); }
            let off = (t + beat * 0.5) % beat; // offbeat bass
            s += 0.25 * (2.0 * PI * hz(root_midi) * t).sin().signum() * (-off * 9.0).exp() * if section != 1 { 1.0 } else { 0.3 };
            let pad: f32 = chord.iter().map(|&m| (2.0 * PI * hz(m) * t).sin()).sum::<f32>() * 0.06;
            s += pad;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let hh = (t + beat * 0.5) % (beat * 0.5);
            if section != 1 { s += 0.05 * ((seed >> 9) as f32 / 4194304.0 - 1.0) * (-hh * 60.0).exp(); }
        }
        let fade = (secs - i as f32 / sr as f32).min(2.0) / 2.0;
        l[i] = s * 0.7 * fade; r[i] = s * 0.7 * fade;
    }
    (l, r)
}

/// Kick-only track (accented downbeats); the tempo glides linearly from `bpm0` to `bpm1` over `secs` (beat phase = integral of tempo).
pub fn synth_drift(bpm0: f32, bpm1: f32, root_midi: i32, minor: bool, secs: f32, lead_in: f32, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let n = (secs * sr as f32) as usize; let mut l = vec![0f32; n]; let mut r = vec![0f32; n];
    let _ = (root_midi, minor);
    let span = (secs - lead_in) as f64;
    for i in 0..n {
        let t = i as f64 / sr as f64 - lead_in as f64;
        let mut s = 0.0f32;
        if t >= 0.0 {
            let ph = (bpm0 as f64 * t + (bpm1 - bpm0) as f64 * t * t / (2.0 * span)) / 60.0; // beats elapsed
            let local = bpm0 as f64 + (bpm1 - bpm0) as f64 * t / span; let beat = 60.0 / local;
            let tb = (ph.fract() * beat) as f32;
            let acc = if (ph as i64) % 4 == 0 { 1.0 } else { 0.55 };
            let f = 50.0 + 120.0 * (-tb * 30.0).exp(); s += 0.8 * acc * (2.0 * PI * f * tb).sin() * (-tb * 7.0).exp();
        }
        let fade = (secs - i as f32 / sr as f32).min(2.0) / 2.0;
        l[i] = s * 0.7 * fade; r[i] = s * 0.7 * fade;
    }
    (l, r)
}

/// Kick track whose kicks before `soft_until` seconds have a soft 55 Hz attack (about 15 ms ramp, no click),
/// like a filtered intro kick; hard clicky kicks after that. Kicks start exactly on the beat.
pub fn synth_soft_intro(bpm: f32, secs: f32, lead_in: f32, soft_until: f32, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let n = (secs * sr as f32) as usize; let mut l = vec![0f32; n]; let beat = 60.0 / bpm;
    for i in 0..n {
        let t = i as f32 / sr as f32 - lead_in; if t < 0.0 { continue; }
        let tb = t % beat;
        let s = if t + lead_in < soft_until { 0.8 * (1.0 - (-tb / 0.006).exp()) * (2.0 * PI * 55.0 * tb).sin() * (-tb * 5.0).exp() }
            else { let f = 50.0 + 120.0 * (-tb * 30.0).exp(); 0.8 * (2.0 * PI * f * tb).sin() * (-tb * 7.0).exp() };
        let fade = (secs - i as f32 / sr as f32).min(2.0) / 2.0; l[i] = s * 0.7 * fade;
    }
    (l.clone(), l)
}

/// Structured test track (bars from `lead_in`): intro 16 (hats + pad), build 16 (hats, snare, riser),
/// drop 32 (kick, bass, hats, lead), breakdown 16 (pad), drop 32, outro 16 (soft kick + hats).
pub fn synth_structured(bpm: f32, lead_in: f32, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let beat = 60.0 / bpm; let bars = 128.0; let secs = lead_in + bars * 4.0 * beat + 1.0;
    let n = (secs * sr as f32) as usize; let mut l = vec![0f32; n];
    let mut seed = 7u32; let mut hp = 0f32; let mut prev = 0f32;
    for i in 0..n {
        let t = i as f32 / sr as f32 - lead_in;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let nz = (seed >> 9) as f32 / 4194304.0 - 1.0; hp = 0.6 * (hp + nz - prev); prev = nz;
        if t < 0.0 || t >= bars * 4.0 * beat { continue; }
        let bar = (t / (beat * 4.0)) as i32; let tb = t % beat; let hh = (t + beat * 0.5) % (beat * 0.5);
        let sec = match bar { 0..=15 => 0, 16..=31 => 1, 32..=63 => 2, 64..=79 => 3, 80..=111 => 2, _ => 4 };
        let pad = (2.0 * PI * 220.0 * t).sin() + (2.0 * PI * 277.2 * t).sin() + (2.0 * PI * 329.6 * t).sin();
        let kick = { let f = 50.0 + 120.0 * (-tb * 30.0).exp(); (2.0 * PI * f * tb).sin() * (-tb * 7.0).exp() };
        let hat = hp * (-hh * 60.0).exp();
        let mut s = 0.0;
        match sec {
            0 => { s += 0.12 * hat + 0.05 * pad; }
            1 => { let p = (bar - 16) as f32 / 16.0; s += 0.12 * hat + 0.05 * pad + 0.35 * hp * (-(t % (beat * 2.0) - beat).abs() * 25.0).exp() + (0.08 + 0.03 * p) * (2.0 * PI * 1200.0 * t).sin(); }
            2 => { let off = (t + beat * 0.5) % beat; s += 0.8 * kick + 0.3 * (2.0 * PI * 55.0 * t).sin().signum() * (-off * 9.0).exp() + 0.15 * hat + 0.06 * pad + 0.1 * (2.0 * PI * 880.0 * t).sin(); }
            3 => { s += 0.1 * pad; }
            _ => { s += 0.4 * kick + 0.12 * hat; }
        }
        l[i] = s * 0.7;
    }
    (l.clone(), l)
}
