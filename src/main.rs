#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod analysis;
mod app;
mod audio;
mod engine;
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--selftest" { std::process::exit(if selftest(Path::new(&args[2])) { 0 } else { 1 }); }
    if args.len() >= 3 && args[1] == "--icon" { std::fs::write(&args[2], app::icon_png(256)).expect("icon"); return; }
    let demo = if args.len() >= 3 && args[1] == "--demo" { Some(args[2].clone()) } else { None };
    // winit has no file drag-and-drop on native Wayland: prefer X11/XWayland when available.
    #[cfg(target_os = "linux")]
    if std::env::var_os("MIXSTUDIO_WAYLAND").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some() {
        std::env::remove_var("WAYLAND_DISPLAY");
    }
    app::run(demo);
}
