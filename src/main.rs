#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod analysis;
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
        mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "Test".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default() });
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
    println!("{}", if ok { "ALL TESTS PASSED" } else { "SOME TEST FAILED" });
    ok
}

fn one_track_mix(a: analysis::Audio, bpm: f64, mix_bpm: f64, key_lock: bool) -> (Arc<engine::Snap>, f64) {
    let dur = a.secs();
    let mut mix = model::Mix::default();
    mix.tracks.push(model::Track { path: "t".into(), title: "t".into(), artist: "t".into(), bpm, offset: 0.0, key: "8A".into(), duration: dur, start_beat: 0.0, lane: 0, trans: Default::default() });
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
        for (k, lane) in [(0usize, 0usize), (1, 1)] { mix.tracks.push(model::Track { path: format!("{k}"), title: "t".into(), artist: "t".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 40.0, start_beat: if k == 0 { 0.0 } else { 40.0 }, lane, trans: Default::default() }); }
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
