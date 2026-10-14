#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod analysis;
mod beatmap;
mod app;
mod audio;
mod engine;
mod stems;
mod model;
mod testgen;
use std::path::Path;
use std::sync::Arc;

/// Generate demo tracks into `dir`, return their paths
pub fn gen_demo(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::create_dir_all(dir).ok();
    // (bpm, root midi, minor, expected camelot)
    let specs = [(128.0, 45, true, "8A", "Kick Test A minor"), (126.0, 48, false, "8B", "Pad Test C major"), (124.0, 40, true, "9A", "Bass Test E minor"), (130.0, 50, true, "7A", "Rave Test D minor")];
    specs.iter().enumerate().map(|(i, &(b, root, mi, _, name))| {
        let p = dir.join(format!("{}.wav", name.replace(' ', "_")));
        if !p.exists() { let (l, r) = testgen::synth(b, root, mi, 150.0, 0.37 * i as f32, 44100); engine::write_wav(&p, &l, &r, 44100).unwrap(); }
        p
    }).collect()
}

fn selftest(dir: &Path) -> bool {
    let paths = gen_demo(dir);
    let expect = [(128.0, "8A", 0.0), (126.0, "8B", 0.37), (124.0, "9A", 0.74), (130.0, "7A", 1.11)];
    let mut ok = true; let mut mix = model::Mix::default(); let mut auds = vec![];
    for (p, &(eb, ek, eo)) in paths.iter().zip(&expect) {
        let t0 = std::time::Instant::now();
        let (a, an) = match analysis::load(p) { Ok(x) => x, Err(e) => { println!("FAIL decode {}: {e}", p.display()); ok = false; continue; } };
        let bar = 4.0 * 60.0 / eb; let od = ((an.offset - eo) / bar).rem_euclid(1.0); let od = od.min(1.0 - od) * bar; // downbeat error (mod bar)
        let pass = (an.bpm - eb).abs() < 0.3 && an.key == ek && od < 0.03;
        ok &= pass;
        println!("{} {}: bpm {:.2} (exp {eb}) key {} (exp {ek}) offset {:.3}s (err {:.3}s, exp {eo}) dur {:.1}s peaks {} [{} ms]",
            if pass { "OK  " } else { "FAIL" }, an.title, an.bpm, an.key, an.offset, od, an.duration, an.peaks.len(), t0.elapsed().as_millis());
        mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "Test".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map() });
        auds.push(Some(Arc::new(a)));
    }
    let presets = [model::Preset::Crossfade, model::Preset::BassSwap, model::Preset::EchoOut, model::Preset::HighToLow];
    for (i, t) in mix.tracks.iter_mut().enumerate() { t.trans.preset = presets[i % 4]; }
    let tr = mix.transitions();
    println!("transitions: {:?}", tr.iter().map(|x| format!("{}->{} bars {:.0}-{:.0}", x.0 + 1, x.1 + 1, x.2 / 4.0, x.3 / 4.0)).collect::<Vec<_>>());
    ok &= tr.len() == 3 && tr.iter().all(|x| ((x.3 - x.2) - 64.0).abs() < 8.0);
    let snap = Arc::new(engine::Snap::build(&mix, &auds));
    let t0 = std::time::Instant::now();
    let (l, r) = engine::render_offline(snap, 44100, 0.0, mix.length_beats());
    let secs = l.len() as f32 / 44100.0;
    let pk = l.iter().chain(&r).fold(0f32, |a, x| a.max(x.abs()));
    let rms = (l.iter().map(|x| x * x).sum::<f32>() / l.len().max(1) as f32).sqrt();
    let nan = l.iter().chain(&r).any(|x| !x.is_finite());
    // silent gaps > 1 s?
    let mut gap = 0usize; let mut maxgap = 0usize;
    for x in &l { if x.abs() < 1e-4 { gap += 1; maxgap = maxgap.max(gap); } else { gap = 0; } }
    println!("render: {secs:.1}s (mix tempo {} BPM) in {} ms, peak {pk:.3}, rms {rms:.4}, nan {nan}, longest silence {:.2}s", mix.bpm, t0.elapsed().as_millis(), maxgap as f32 / 44100.0);
    ok &= !nan && pk > 0.1 && pk <= 1.0 && rms > 0.02 && (maxgap as f32 / 44100.0) < 1.0;
    engine::write_wav(&dir.join("render.wav"), &l, &r, 44100).ok();
    let js = serde_json::to_string_pretty(&mix).unwrap();
    let back: model::Mix = serde_json::from_str(&js).unwrap();
    ok &= back.tracks.len() == mix.tracks.len();
    std::fs::write(dir.join("demo.mixstudio.json"), js).ok();
    ok &= stretch_tests();
    ok &= stem_tests();
    ok &= alignment_test(dir);
    ok &= drift_test(dir);
    println!("{}", if ok { "ALL TESTS PASSED" } else { "SOME TEST FAILED" });
    ok
}

fn one_track_mix(a: analysis::Audio, bpm: f64, mix_bpm: f64, key_lock: bool) -> (Arc<engine::Snap>, f64) {
    let dur = a.secs();
    let mut mix = model::Mix::default();
    mix.tracks.push(model::Track { path: "t".into(), title: "t".into(), artist: "t".into(), bpm, offset: 0.0, key: "8A".into(), duration: dur, start_beat: 0.0, lane: 0, trans: Default::default(), map: Option::None });
    mix.bpm = mix_bpm; mix.key_lock = key_lock; mix.metronome = true; // metronome must stay out of offline renders
    let len = mix.length_beats();
    (Arc::new(engine::Snap::build(&mix, &[Some(Arc::new(a))])), len)
}
fn freq(x: &[f32], sr: f32) -> f32 { let mut z = 0; for w in x.windows(2) { if w[0] < 0.0 && w[1] >= 0.0 { z += 1; } } z as f32 * sr / x.len() as f32 }

fn stretch_tests() -> bool {
    let sr = 44100u32; let mut ok = true;
    let sine: Vec<f32> = (0..sr as usize * 20).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin()).collect();
    for (kl, expect) in [(true, 440.0f32), (false, 462.0)] {
        let (snap, _) = one_track_mix(analysis::Audio { sr, l: sine.clone(), r: sine.clone() }, 120.0, 126.0, kl);
        let (l, _) = engine::render_offline(snap, sr, 4.0, 28.0);
        let f = freq(&l, sr as f32);
        let rms = (l.iter().map(|x| x * x).sum::<f32>() / l.len() as f32).sqrt();
        let exp_len = (24.0 * 60.0 / 126.0 * sr as f64) as usize;
        let pass = (f - expect).abs() < 3.0 && (rms - 0.5 * 0.85 / 2f32.sqrt()).abs() < 0.06 && l.len() == exp_len;
        ok &= pass;
        println!("{} stretch 440 Hz sine 120->126 BPM (ratio 1.05), key lock {}: {:.1} Hz (exp {expect}), rms {:.3}, length {} (exp {exp_len})", if pass { "OK  " } else { "FAIL" }, if kl { "on " } else { "off" }, f, rms, l.len());
    }
    let (kl, kr) = testgen::synth(128.0, 45, true, 60.0, 0.0, sr);
    let (snap, len) = one_track_mix(analysis::Audio { sr, l: kl, r: kr }, 128.0, 134.4, true);
    let t0 = std::time::Instant::now();
    let (l, _) = engine::render_offline(snap, sr, 0.0, len);
    let el = t0.elapsed().as_secs_f64(); let secs = l.len() as f64 / sr as f64;
    let (bpm, db) = analysis::detect_bpm(&l, sr);
    let exp_secs = 60.0 * 128.0 / 60.0 * 60.0 / 134.4;
    let pass = (bpm - 134.4).abs() < 0.3 && (secs - exp_secs).abs() < 0.05;
    ok &= pass;
    println!("{} stretch kick track 128->134.4 BPM: detected {:.2} BPM, downbeat {:.3}s, length {:.2}s (exp {:.2}s); WSOLA speed {:.0}x real time per track ({:.2}% of one core)",
        if pass { "OK  " } else { "FAIL" }, bpm, db, secs, exp_secs, secs / el, 100.0 * el / secs);
    ok
}

fn goertzel(x: &[f32], f: f32, sr: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * f / sr; let c = 2.0 * w.cos(); let (mut s1, mut s2) = (0f32, 0f32);
    for &v in x { let s0 = v + c * s1 - s2; s2 = s1; s1 = s0; }
    ((s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0)).sqrt() / x.len() as f32 * 2.0
}

fn stem_tests() -> bool {
    use model::StemMode::*;
    let mut ok = true;
    // 1) gain logic
    let g = |m, x, o| engine::stem_gain(m, x, o);
    let checks = [(None, 0.3, true, 0.0), (None, 0.7, false, 0.0), (Full, 0.9, true, 1.0), (Full, 0.1, false, 1.0),
        (Swap, 0.2, true, 1.0), (Swap, 0.8, true, 0.0), (Swap, 0.2, false, 0.0), (Swap, 0.8, false, 1.0),
        (Crossfade, 0.0, true, 1.0), (Crossfade, 1.0, false, 1.0), (Crossfade, 0.5, true, 0.7071)];
    let bad: Vec<_> = checks.iter().filter(|c| (g(c.0, c.1, c.2) - c.3).abs() > 0.01).collect();
    let cf_pow = (0..=10).map(|k| { let x = k as f32 / 10.0; g(Crossfade, x, true).powi(2) + g(Crossfade, x, false).powi(2) }).all(|p| (p - 1.0).abs() < 1e-3);
    let pass = bad.is_empty() && cf_pow; ok &= pass;
    println!("{} stem gain logic: {} cases, crossfade equal-power {}", if pass { "OK  " } else { "FAIL" }, checks.len(), cf_pow);

    // 2) per-stem transition in the engine with synthetic stems: A = drums 100 Hz, B = vocals 1000 Hz
    let sr = 44100u32; let n = sr as usize * 40;
    let tone = |f: f32| -> Vec<f32> { (0..n).map(|i| 0.3 * (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32).sin()).collect() };
    let (ta, tb) = (tone(100.0), tone(1000.0));
    let mk = |sig: &Vec<f32>, k: usize| { let q: Vec<i16> = sig.iter().map(|x| (x * 32767.0) as i16).collect(); let z = vec![0i16; n];
        let mut st = stems::Stems { d: [z.clone(), z.clone()], b: [z.clone(), z.clone()], v: [z.clone(), z.clone()], peaks: vec![] };
        match k { 0 => st.d = [q.clone(), q], _ => st.v = [q.clone(), q] }; std::sync::Arc::new(st) };
    let mut res = vec![];
    for (dm, vm) in [(Full, None), (None, Full)] {
        let mut mix = model::Mix::default(); mix.bpm = 120.0;
        for (k, lane) in [(0usize, 0usize), (1, 1)] { mix.tracks.push(model::Track { path: format!("{k}"), title: "t".into(), artist: "t".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 40.0, start_beat: if k == 0 { 0.0 } else { 40.0 }, lane, trans: Default::default(), map: Option::None }); }
        mix.tracks[1].trans.stems = [dm, Crossfade, Crossfade, vm];
        let auds = vec![Some(Arc::new(analysis::Audio { sr, l: ta.clone(), r: ta.clone() })), Some(Arc::new(analysis::Audio { sr, l: tb.clone(), r: tb.clone() }))];
        let st = vec![Some(mk(&ta, 0)), Some(mk(&tb, 3))];
        let snap = Arc::new(engine::Snap::build_with(&mix, &auds, &st));
        // overlap is beats 40..80; measure at x = 0.75 (beats 68..72)
        let (l, _) = engine::render_offline(snap, sr, 68.0, 72.0);
        res.push((goertzel(&l, 100.0, sr as f32), goertzel(&l, 1000.0, sr as f32)));
    }
    let pass = res[0].0 > 0.15 && res[0].1 < 0.01 && res[1].0 < 0.01 && res[1].1 > 0.15; ok &= pass;
    println!("{} per-stem transition: drums Full/vocals None -> 100 Hz {:.3}, 1 kHz {:.3}; drums None/vocals Full -> 100 Hz {:.3}, 1 kHz {:.3}",
        if pass { "OK  " } else { "FAIL" }, res[0].0, res[0].1, res[1].0, res[1].1);

    // 3) real model (only if ONNX Runtime and the model are available)
    match stems::session_info() {
        Err(e) => println!("SKIP model separation test: {e}"),
        Ok(ep) => {
            if !stems::verify_model() { println!("FAIL model checksum"); return false; }
            let secs = 16usize; let n = sr as usize * secs;
            let mut kick = vec![0f32; n]; let mut pad = vec![0f32; n];
            for i in 0..n { let t = i as f32 / sr as f32; let tb = t % 0.5; let f = 50.0 + 120.0 * (-tb * 30.0).exp();
                kick[i] = 0.6 * (2.0 * std::f32::consts::PI * f * tb).sin() * (-tb * 7.0).exp();
                pad[i] = [220.0f32, 277.2, 329.6].iter().map(|f| (2.0 * std::f32::consts::PI * f * t).sin()).sum::<f32>() * 0.08; }
            let mixs: Vec<f32> = kick.iter().zip(&pad).map(|(a, b)| a + b).collect();
            let a = analysis::Audio { sr, l: mixs.clone(), r: mixs.clone() };
            let p = std::sync::atomic::AtomicU32::new(0); let t0 = std::time::Instant::now();
            match stems::separate_raw(&a, &p) {
                Err(e) => { println!("FAIL separation: {e}"); ok = false; }
                Ok(raw) => {
                    let el = t0.elapsed().as_secs_f32();
                    let sum: Vec<f32> = (0..n).map(|i| raw.iter().map(|s| s[0][i]).sum()).collect();
                    let e_mix = mixs.iter().map(|x| x * x).sum::<f32>(); let e_err = sum.iter().zip(&mixs).map(|(a, b)| (a - b).powi(2)).sum::<f32>();
                    let corr = |x: &[f32], y: &[f32]| { let (mut a, mut b, mut c) = (0f32, 0f32, 0f32); for (p, q) in x.iter().zip(y) { a += p * q; b += p * p; c += q * q; } a / (b * c).sqrt().max(1e-9) };
                    let cd = corr(&raw[0][0], &kick); let cp = corr(&raw[2][0], &pad).max(corr(&raw[3][0], &pad));
                    let rel = (e_err / e_mix).sqrt();
                    let pass = rel < 0.25 && cd > 0.7; ok &= pass;
                    println!("{} HT-Demucs on {ep}: {secs}s audio in {el:.1}s ({:.1}x real time); sum of stems vs original: rel. error {:.3}; corr(drums, kick) {:.2}, corr(other/vocals, pad) {:.2}",
                        if pass { "OK  " } else { "FAIL" }, secs as f32 / el, rel, cd, cp);
                }
            }
        }
    }
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--analyse" { for p in &args[2..] { let pa = Path::new(p); match analysis::decode(pa) { Ok((a, t, ar)) => { let an = analysis::analyse(&a, t, ar, pa); println!("{p}: bpm {:.3} downbeat {:.3}s key {}", an.bpm, an.offset, an.key); } Err(e) => println!("{p}: {e}") } } return; }
    if args.len() >= 4 && args[1] == "--align" { align_cli(Path::new(&args[2]), Path::new(&args[3])); return; }
    if args.len() >= 3 && args[1] == "--selftest" { std::process::exit(if selftest(Path::new(&args[2])) { 0 } else { 1 }); }
    if args.len() >= 3 && args[1] == "--separate" {
        for f in &args[2..] { let p = Path::new(f); let (a, _) = analysis::load(p).expect("decode");
            let key = analysis::cache_key(p).unwrap(); let (dl, pr) = (std::sync::atomic::AtomicU32::new(0), std::sync::atomic::AtomicU32::new(0));
            let t0 = std::time::Instant::now();
            match stems::get_or_separate(&key, &a, &dl, &pr) { Ok(_) => println!("{}: stems ready in {:.1}s ({})", f, t0.elapsed().as_secs_f32(), stems::session_info().unwrap_or_default()), Err(e) => println!("{f}: {e}") } }
        return;
    }
    if args.len() >= 3 && args[1] == "--icon" { std::fs::write(&args[2], app::icon_png(256)).expect("icon"); return; }
    let demo = if args.len() >= 3 && args[1] == "--demo" { Some(args[2].clone()) } else { None };
    // winit has no file drag-and-drop on native Wayland: prefer X11/XWayland when available.
    #[cfg(target_os = "linux")]
    if std::env::var_os("MIXSTUDIO_WAYLAND").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some() {
        std::env::remove_var("WAYLAND_DISPLAY");
    }
    app::run(demo);
}

/// Kick onset times (s) in a render: first sample after each expected beat where |x| crosses 30% of that beat window's peak.
fn kick_onsets(x: &[f32], sr: u32, beat_s: f64, first: f64) -> Vec<Option<f64>> {
    let mut v = vec![]; let mut t = first;
    while ((t + beat_s) * sr as f64) < x.len() as f64 {
        let s = ((t - beat_s * 0.4).max(0.0) * sr as f64) as usize; let e = ((t + beat_s * 0.4) * sr as f64) as usize;
        let w = &x[s..e.min(x.len())]; let pk = w.iter().fold(0f32, |a, y| a.max(y.abs()));
        v.push(if pk < 0.01 { None } else { w.iter().position(|y| y.abs() > 0.3 * pk).map(|i| (s + i) as f64 / sr as f64) });
        t += beat_s;
    }
    v
}

/// Two kick tracks at 124 and 128 BPM, analysed, laid out by the mixer and rendered at the 126 BPM mix tempo:
/// every kick of the outgoing track must land within 10 ms of the incoming track's kick across the whole overlap.
fn alignment_test(dir: &Path) -> bool {
    let sr = 44100u32; let mut ok = true;
    for key_lock in [true, false] {
        let mut mix = model::Mix::default(); let mut auds = vec![];
        for (i, &(bpm, lead)) in [(124.0f32, 0.53f32), (128.0, 0.21)].iter().enumerate() {
            let p = dir.join(format!("align_{}.wav", bpm as i32));
            let (l, r) = testgen::synth(bpm, 45 + i as i32 * 3, true, 90.0, lead, sr); engine::write_wav(&p, &l, &r, sr).unwrap();
            let (a, an) = analysis::load(&p).unwrap();
            mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "t".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map() });
            auds.push(Some(Arc::new(a)));
        }
        mix.bpm = 126.0; mix.key_lock = key_lock;
        let tr = mix.transitions(); if tr.is_empty() { println!("FAIL align: no overlap"); return false; }
        let (_, _, s, e) = tr[0];
        let solo = |d: usize| { let mut m = mix.clone(); m.decks[1 - d].mute = true;
            let snap = Arc::new(engine::Snap::build(&m, &auds)); engine::render_offline(snap, sr, s, e).0 };
        let (a, b) = (solo(0), solo(1));
        let beat_s = 60.0 / mix.bpm;
        let (oa, ob) = (kick_onsets(&a, sr, beat_s, 0.0), kick_onsets(&b, sr, beat_s, 0.0));
        let d: Vec<f64> = oa.iter().zip(&ob).filter_map(|(x, y)| Some((x.as_ref()? - y.as_ref()?).abs() * 1000.0)).collect();
        let max = d.iter().cloned().fold(0.0, f64::max); let mean = d.iter().sum::<f64>() / d.len().max(1) as f64;
        let pass = d.len() >= 32 && max < 10.0;
        ok &= pass;
        println!("{} align 124+128 BPM kicks at 126 BPM, key lock {}: analysed {:.3}/{:.3} BPM, {} kick pairs over {:.0} beats, offset mean {:.2} ms, max {:.2} ms (limit 10 ms)",
            if pass { "OK  " } else { "FAIL" }, if key_lock { "on " } else { "off" }, mix.tracks[0].bpm, mix.tracks[1].bpm, d.len(), e - s, mean, max);
    }
    ok
}

fn track_of(p: &Path, an: &analysis::Analysis) -> model::Track {
    model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "t".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map() }
}

fn lowpass(x: &[f32], sr: u32) -> Vec<f32> {
    let a = (-2.0 * std::f32::consts::PI * 150.0 / sr as f32).exp(); let (mut y1, mut y2) = (0f32, 0f32);
    x.iter().map(|&v| { y1 = a * y1 + (1.0 - a) * v; y2 = a * y2 + (1.0 - a) * y1; y2 }).collect()
}

/// Render each deck of the first transition on its own and compare kick onsets beat by beat.
/// Returns (pairs, mean ms, median ms, max ms, overlap beats).
fn overlap_alignment(mix: &model::Mix, auds: &[Option<Arc<analysis::Audio>>], sr: u32, real: bool) -> Option<(usize, f64, f64, f64, f64)> {
    let tr = mix.transitions(); let &(_, _, s, e) = tr.first()?;
    let solo = |d: usize| { let mut m = mix.clone(); m.decks[1 - d].mute = true; m.metronome = false;
        let snap = Arc::new(engine::Snap::build(&m, auds)); let x = engine::render_offline(snap, sr, s, e).0; if real { lowpass(&x, sr) } else { x } };
    let (a, b) = (solo(0), solo(1));
    if let Ok(dd) = std::env::var("MS_DUMP") { engine::write_wav(&Path::new(&dd).join("deck_a.wav"), &a, &a, sr).ok(); engine::write_wav(&Path::new(&dd).join("deck_b.wav"), &b, &b, sr).ok(); }
    let beat_s = 60.0 / mix.bpm;
    let (oa, ob) = (kick_onsets(&a, sr, beat_s, 0.0), kick_onsets(&b, sr, beat_s, 0.0));
    if std::env::var("MS_DEBUG").is_ok() { for (k, (x, y)) in oa.iter().zip(&ob).enumerate() { println!("  beat {k}: {:?} {:?}", x.map(|v| (v * 1000.0).round()), y.map(|v| (v * 1000.0).round())); } }
    let mut d: Vec<f64> = oa.iter().zip(&ob).filter_map(|(x, y)| Some((x.as_ref()? - y.as_ref()?).abs() * 1000.0)).collect();
    if real { d.retain(|&x| x < 0.25 * beat_s * 1000.0); } // drop beats where one side has no kick (breaks, fills)
    if d.is_empty() { return None; }
    let mean = d.iter().sum::<f64>() / d.len() as f64; let max = d.iter().cloned().fold(0.0, f64::max);
    let mut s2 = d.clone(); s2.sort_by(|a, b| a.partial_cmp(b).unwrap()); let med = s2[s2.len() / 2];
    Some((d.len(), mean, med, max, e - s))
}

/// Drifting kick track (125.05 -> 125.7 BPM) against a steady 124 BPM track, both orders, at 125 BPM:
/// kicks must stay within 10 ms over the whole overlap. Also prints the fixed-grid result for comparison.
fn drift_test(dir: &Path) -> bool {
    let sr = 44100u32; let mut ok = true;
    let pd = dir.join("drift_125.05-125.7.wav"); let ps = dir.join("steady_124.wav");
    let (l, r) = testgen::synth_drift(125.05, 125.7, 45, true, 150.0, 0.41, sr); engine::write_wav(&pd, &l, &r, sr).unwrap();
    let (l, r) = testgen::synth_drift(124.0, 124.0, 48, true, 150.0, 0.23, sr); engine::write_wav(&ps, &l, &r, sr).unwrap();
    let (ad, and) = analysis::load(&pd).unwrap(); let (as_, ans) = analysis::load(&ps).unwrap();
    let (ad, as_) = (Arc::new(ad), Arc::new(as_));
    if let Some(m) = and.beat_map() { let (lo, hi) = m.tempo_range(); println!("     drift track: {} beats mapped, global {:.2} BPM, local {:.2}..{:.2} BPM, downbeat {:.3}s (exp 0.41 mod bar)", m.beats.len(), and.bpm, lo, hi, and.offset); }
    if let Some(m) = and.beat_map() {
        // true beat times of the glide: phase(t) = (b0 t + (b1-b0) t^2 / 2T) / 60
        let (b0, b1, lead) = (125.05f64, 125.7f64, 0.41f64); let span = 150.0 - lead; let c = (b1 - b0) / span;
        let truth = |k: f64| lead + (-b0 + (b0 * b0 + 2.0 * c * 60.0 * k).sqrt()) / c;
        let errs: Vec<f64> = m.beats.iter().map(|&t| { let k = ((t - lead) * (b0 + c * (t - lead) / 2.0) / 60.0).round(); (t - truth(k)) * 1000.0 }).collect();
        let max = errs.iter().fold(0f64, |a, e| a.max(e.abs())); let mean = errs.iter().sum::<f64>() / errs.len() as f64;
        let pass = max < 5.0; ok &= pass;
        println!("{} drift beat map vs true beat times: mean {mean:+.2} ms, max |err| {max:.2} ms over {} beats (limit 5 ms)", if pass { "OK  " } else { "FAIL" }, errs.len());
        if std::env::var("MS_DEBUG").is_ok() { for (k, e) in errs.iter().enumerate().take(24) { println!("  map beat {k}: {e:+.2} ms"); } }
    }
    for (order, key_lock) in [(0, true), (1, true), (0, false)] {
        for dynamic in [true, false] {
            let mut mix = model::Mix::default(); let mut auds = vec![];
            let mut add = |p: &Path, an: &analysis::Analysis, a: &Arc<analysis::Audio>| { let mut t = track_of(p, an); if !dynamic { t.map = None; } mix.add_track(t); auds.push(Some(a.clone())); };
            if order == 0 { add(&pd, &and, &ad); add(&ps, &ans, &as_); } else { add(&ps, &ans, &as_); add(&pd, &and, &ad); }
            mix.bpm = 125.0; mix.key_lock = key_lock;
            let Some((n, mean, med, max, len)) = overlap_alignment(&mix, &auds, sr, false) else { println!("FAIL drift: no overlap"); ok = false; continue; };
            let name = if order == 0 { "drift->steady" } else { "steady->drift" };
            if dynamic {
                let pass = n >= 48 && max < 10.0; ok &= pass;
                println!("{} beatmap align {name}, key lock {}: {n} kick pairs over {len:.0} beats, mean {mean:.2} ms, median {med:.2} ms, max {max:.2} ms (limit 10 ms)", if pass { "OK  " } else { "FAIL" }, if key_lock { "on " } else { "off" });
            } else {
                println!("INFO fixed-grid comparison {name}, key lock {}: mean {mean:.2} ms, max {max:.2} ms", if key_lock { "on " } else { "off" });
            }
        }
    }
    // a project saved without a beat map still loads and uses the linear grid
    let mut t = track_of(&ps, &ans); t.map = None;
    let js = serde_json::to_string(&t).unwrap().replace(",\"map\":null", "");
    let back: Result<model::Track, _> = serde_json::from_str(&js);
    let pass = back.as_ref().is_ok_and(|b| b.map.is_none() && (b.sec_at(b.start_beat + 8.0) - (b.offset + 8.0 * 60.0 / b.bpm)).abs() < 1e-9);
    ok &= pass; println!("{} legacy project without beat map loads with linear grid", if pass { "OK  " } else { "FAIL" });
    ok
}

/// `mixstudio --align a b`: analyse two files, lay them out as a mix and measure kick alignment in the rendered overlap.
fn align_cli(a: &Path, b: &Path) {
    let sr = 44100u32;
    let (aa, ana) = analysis::load(a).unwrap(); let (ab, anb) = analysis::load(b).unwrap();
    let (aa, ab) = (Arc::new(aa), Arc::new(ab));
    for (an, p) in [(&ana, a), (&anb, b)] { let r = an.beat_map().map(|m| m.tempo_range()).unwrap_or((an.bpm, an.bpm)); println!("{}: global {:.3} BPM, local {:.2}..{:.2}, downbeat {:.3}s, {} beats", p.display(), an.bpm, r.0, r.1, an.offset, an.beats.len()); }
    for dynamic in [false, true] { for key_lock in [false, true] {
        let mut mix = model::Mix::default(); let mut ta = track_of(a, &ana); let mut tb = track_of(b, &anb);
        if !dynamic { ta.map = None; tb.map = None; }
        mix.add_track(ta); mix.add_track(tb); mix.bpm = ((ana.bpm + anb.bpm) / 2.0 * 10.0).round() / 10.0; mix.key_lock = key_lock;
        match overlap_alignment(&mix, &[Some(aa.clone()), Some(ab.clone())], sr, true) {
            Some((n, mean, med, max, len)) => println!("{} grid, key lock {}, mix {:.1} BPM: {n} kick pairs over {len:.0} beats, mean {mean:.2} ms, median {med:.2} ms, max {max:.2} ms",
                if dynamic { "beat-map" } else { "fixed   " }, if key_lock { "on " } else { "off" }, mix.bpm),
            None => println!("no overlap"),
        }
    } }
}
