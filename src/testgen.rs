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
