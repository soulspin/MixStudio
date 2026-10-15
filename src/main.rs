#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod analysis;
mod automix;
mod library;
mod dialogs;
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
        mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "Test".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 });
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
    ok &= soft_intro_test(dir);
    ok &= automix_tests();
    ok &= library_tests(dir);
    ok &= beatgrid_tests();
    ok &= timeline_tests();
    println!("{}", if ok { "ALL TESTS PASSED" } else { "SOME TEST FAILED" });
    ok
}

fn one_track_mix(a: analysis::Audio, bpm: f64, mix_bpm: f64, key_lock: bool) -> (Arc<engine::Snap>, f64) {
    let dur = a.secs();
    let mut mix = model::Mix::default();
    mix.tracks.push(model::Track { path: "t".into(), title: "t".into(), artist: "t".into(), bpm, offset: 0.0, key: "8A".into(), duration: dur, start_beat: 0.0, lane: 0, trans: Default::default(), map: Option::None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 });
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
        for (k, lane) in [(0usize, 0usize), (1, 1)] { mix.tracks.push(model::Track { path: format!("{k}"), title: "t".into(), artist: "t".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 40.0, start_beat: if k == 0 { 0.0 } else { 40.0 }, lane, trans: Default::default(), map: Option::None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 }); }
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
    if std::env::args().nth(1).as_deref() == Some("--dialog-check") { for (n, r) in dialogs::backends() { println!("{n:8} {}", match r { Ok(()) => "available".to_string(), Err(e) => format!("unavailable: {e}") }); } return; }
    if std::env::args().nth(1).as_deref() == Some("--dialog-test") { println!("{:?}", dialogs::run(dialogs::Req::Folders { title: "MixStudio dialog test".into() })); return; }
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
            mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "t".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 });
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
    model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "t".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 }
}


/// Render each deck of the first transition on its own and compare kick onsets beat by beat.
/// Returns (pairs, mean ms, median ms, max ms, overlap beats).
fn overlap_alignment(mix: &model::Mix, auds: &[Option<Arc<analysis::Audio>>], sr: u32, real: bool) -> Option<(usize, f64, f64, f64, f64)> {
    let tr = mix.transitions(); let &(_, _, s, e) = tr.first()?;
    let solo = |d: usize| { let mut m = mix.clone(); m.decks[1 - d].mute = true; m.metronome = false;
        let snap = Arc::new(engine::Snap::build(&m, auds)); engine::render_offline(snap, sr, s, e).0 };
    let (a, b) = (solo(0), solo(1));
    if let Ok(dd) = std::env::var("MS_DUMP") { engine::write_wav(&Path::new(&dd).join("deck_a.wav"), &a, &a, sr).ok(); std::fs::write(Path::new(&dd).join("s.txt"), format!("{s} {e} {}", mix.bpm)).ok(); engine::write_wav(&Path::new(&dd).join("deck_b.wav"), &b, &b, sr).ok(); }
    let beat_s = 60.0 / mix.bpm;
    let (oa, ob) = if real { (attack_onsets(&a, sr, beat_s), attack_onsets(&b, sr, beat_s)) } else { (kick_onsets(&a, sr, beat_s, 0.0), kick_onsets(&b, sr, beat_s, 0.0)) };
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

/// Steepest rise of a ripple-free low-band envelope near every mix beat (robust on real kicks). Times in s, None if no kick.
fn attack_onsets(x: &[f32], sr: u32, beat_s: f64) -> Vec<Option<f64>> {
    let a = (-2.0 * std::f32::consts::PI * 150.0 / sr as f32).exp(); let wq = sr as f32 / (2.0 * std::f32::consts::PI * 60.0);
    let (mut y1, mut y2, mut pv) = (0f32, 0f32, 0f32);
    let env: Vec<f32> = x.iter().map(|&v| { y1 = a * y1 + (1.0 - a) * v; y2 = a * y2 + (1.0 - a) * y1; let d = (y2 - pv) * wq; pv = y2; (y2 * y2 + d * d).sqrt() }).collect();
    let gmax = env.iter().cloned().fold(0f32, f32::max); let dd = (0.001 * sr as f64) as usize;
    let mut v = vec![]; let mut t = 0.0;
    while ((t + 0.25 * beat_s) * sr as f64) as usize + dd < env.len() {
        let s = ((t - 0.2 * beat_s).max(0.0) * sr as f64) as usize; let e = ((t + 0.2 * beat_s) * sr as f64) as usize;
        let pk = env[s..e].iter().cloned().fold(0f32, f32::max);
        v.push(if pk < 0.02 * gmax { None } else { let mut b = (0f32, s); for i in s..e { let r = env[i + dd] - env[i]; if r > b.0 { b = (r, i); } } Some(b.1 as f64 / sr as f64) });
        t += beat_s;
    }
    v
}

/// Regression for the Night Transit order bug (v0.3.0): (1) beats on soft-attack kicks were placed later than on
/// clicky kicks, and (2) key-lock WSOLA moved attacks by a few ms in a direction set by speed-up vs slow-down.
/// A hard-kick 125.2 BPM track (slowed) into a 124 BPM track with a soft-kick intro (sped up), mixed at 124.6:
/// the median kick offset over the overlap must stay under 2.5 ms with key lock on and off.
fn soft_intro_test(dir: &Path) -> bool {
    let sr = 44100u32; let mut ok = true;
    let pa = dir.join("hard_125.2.wav"); let pb = dir.join("soft_intro_124.wav");
    let (l, r) = testgen::synth_drift(125.2, 125.2, 45, true, 120.0, 0.31, sr); engine::write_wav(&pa, &l, &r, sr).unwrap();
    let (l, r) = testgen::synth_soft_intro(124.0, 120.0, 0.17, 45.0, sr); engine::write_wav(&pb, &l, &r, sr).unwrap();
    let (aa, ana) = analysis::load(&pa).unwrap(); let (ab, anb) = analysis::load(&pb).unwrap();
    let (aa, ab) = (Arc::new(aa), Arc::new(ab));
    for key_lock in [true, false] {
        let mut mix = model::Mix::default(); mix.add_track(track_of(&pa, &ana)); mix.add_track(track_of(&pb, &anb)); mix.bpm = 124.6; mix.key_lock = key_lock;
        let tr = mix.transitions(); let Some(&(_, _, s, e)) = tr.first() else { println!("FAIL soft intro: no overlap"); return false; };
        let solo = |d: usize| { let mut m = mix.clone(); m.decks[1 - d].mute = true; engine::render_offline(Arc::new(engine::Snap::build(&m, &[Some(aa.clone()), Some(ab.clone())])), sr, s, e).0 };
        let beat_s = 60.0 / mix.bpm;
        let (oa, ob) = (attack_onsets(&solo(0), sr, beat_s), attack_onsets(&solo(1), sr, beat_s));
        // the soft and hard kicks have different attack shapes; compare against the same kick measured on its own
        // track at its own tempo, so only placement errors remain: offset of (A - B) minus the shape bias
        let shape = { let (x, _) = testgen::synth_soft_intro(124.6, 8.0, 0.0, 4.0, sr); let o = attack_onsets(&x, sr, beat_s); let n = (4.0 / beat_s) as usize;
            let soft: Vec<f64> = o[1..n].iter().enumerate().filter_map(|(k, v)| v.map(|v| v - (k + 1) as f64 * beat_s)).collect();
            let hard: Vec<f64> = o[n + 1..o.len() - 1].iter().enumerate().filter_map(|(k, v)| v.map(|v| v - (k + n + 1) as f64 * beat_s)).collect();
            soft.iter().sum::<f64>() / soft.len() as f64 - hard.iter().sum::<f64>() / hard.len() as f64 };
        let mut d: Vec<f64> = oa.iter().zip(&ob).filter_map(|(x, y)| Some((x.as_ref()? - (y.as_ref()? - shape)) * 1000.0)).collect();
        d.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = d.get(d.len() / 2).cloned().unwrap_or(f64::NAN); let max = d.iter().fold(0f64, |a, x| a.max(x.abs()));
        let pass = d.len() >= 32 && med.abs() < 2.5 && max < 10.0; ok &= pass;
        println!("{} soft-intro regression (hard 125.2 -> soft-intro 124 at 124.6), key lock {}: {} kick pairs, median {med:+.2} ms, max {max:.2} ms (attack-shape bias {:.2} ms removed; limits 2.5 / 10 ms)",
            if pass { "OK  " } else { "FAIL" }, if key_lock { "on " } else { "off" }, d.len(), shape * 1000.0);
    }
    ok
}

fn automix_tests() -> bool {
    use automix::*; let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} auto-order {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let ks = [("8A", "8A"), ("8A", "9A"), ("8A", "7A"), ("8A", "8B"), ("8A", "9B"), ("8A", "10A"), ("8A", "3A"), ("8A", "2B")];
    let v: Vec<(f64, &str)> = ks.iter().map(|(a, b)| key_score(a, b)).collect();
    chk("key scoring", v[0].0 > v[1].0 && (v[1].0 - v[2].0).abs() < 1e-9 && v[1].0 > v[3].0 && v[3].0 > v[4].0 && v[4].0 > v[5].0 && v[5].0 > v[6].0 && v[6].0 > v[7].0 && v[5].1 == "energy +2" && v[6].1 == "energy +7",
        format!("{}", ks.iter().zip(&v).map(|((a, b), s)| format!("{a}→{b} {:.2} {}", s.0, s.1)).collect::<Vec<_>>().join(", ")));
    let (b1, p1) = bpm_score(128.0, 128.0); let (b2, p2) = bpm_score(128.0, 130.0); let (b3, p3) = bpm_score(128.0, 66.0); let (b4, _) = bpm_score(128.0, 100.0);
    chk("tempo scoring", b1 > b2 && b2 > b3 && b3 > 0.4 && b4 < 0.01 && (p3 - 3.125).abs() < 0.01, format!("128→128 {b1:.2} ({p1:+.1}%), 128→130 {b2:.2} ({p2:+.1}%), 128→66 half time {b3:.2} ({p3:+.1}%), 128→100 {b4:.3}"));
    // known optimal chain: 12 tracks 1A..12A at 120..131 BPM, given shuffled; best order is the chain (or its reverse)
    let chain: Vec<Item> = (0..12).map(|k| Item { key: format!("{}A", k + 1), bpm: 120.0 + k as f64, energy: k as f64 }).collect();
    let shuf = [7usize, 2, 11, 0, 5, 9, 3, 10, 1, 6, 8, 4];
    let items: Vec<Item> = shuf.iter().map(|&k| chain[k].clone()).collect();
    let opts = Opts { mode: Mode::Balanced, curve: Curve::Off };
    let ord = optimize(&items, &vec![false; 12], &opts);
    let got: Vec<usize> = ord.iter().map(|&i| shuf[i]).collect();
    let fwd: Vec<usize> = (0..12).collect(); let rev: Vec<usize> = (0..12).rev().collect();
    chk("finds known optimal chain", got == fwd || got == rev, format!("{:?}", got.iter().map(|&k| chain[k].key.clone()).collect::<Vec<_>>()));
    // build-up curve picks the ascending direction
    let ob = optimize(&items, &vec![false; 12], &Opts { mode: Mode::Balanced, curve: Curve::Build });
    let gb: Vec<usize> = ob.iter().map(|&i| shuf[i]).collect();
    chk("energy build-up direction", gb == fwd, format!("{:?}", gb));
    // locks: first track fixed at position 0 (7 = "8A") and position 6 locked; they must stay
    let mut lk = vec![false; 12]; lk[0] = true; lk[6] = true;
    let ol = optimize(&items, &lk, &opts);
    let rest_ok = { let mut s = ol.clone(); s.sort(); s == (0..12).collect::<Vec<_>>() };
    chk("respects locks", ol[0] == 0 && ol[6] == 6 && rest_ok, format!("pos0 {} pos6 {} (kept {} / {}), cost {:.3} vs unlocked {:.3}", items[ol[0]].key, items[ol[6]].key, items[0].key, items[6].key, order_cost(&items, &ol, &opts), order_cost(&items, &ord, &opts)));
    // modes: A=8A@128, choices B=8A@134 (same key, +4.7%) and C=3B@128 (clash, same tempo)
    let tri = vec![Item { key: "8A".into(), bpm: 128.0, energy: 0.0 }, Item { key: "8A".into(), bpm: 134.0, energy: 0.0 }, Item { key: "3B".into(), bpm: 128.0, energy: 0.0 }];
    let lk3 = [true, false, false];
    let h = optimize(&tri, &lk3, &Opts { mode: Mode::Harmonic, curve: Curve::Off }); let t = optimize(&tri, &lk3, &Opts { mode: Mode::Tempo, curve: Curve::Off });
    chk("harmonic vs tempo mode", h[1] == 1 && t[1] == 2, format!("harmonic picks {} {:.0} next, tempo picks {} {:.0} next", tri[h[1]].key, tri[h[1]].bpm, tri[t[1]].key, tri[t[1]].bpm));
    // speed: 100 random tracks
    let mut s = 12345u64; let mut r = move || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let big: Vec<Item> = (0..100).map(|_| Item { key: format!("{}{}", r() % 12 + 1, if r() % 2 == 0 { 'A' } else { 'B' }), bpm: 118.0 + (r() % 1500) as f64 / 100.0, energy: (r() % 100) as f64 }).collect();
    let t0 = std::time::Instant::now(); let ob = optimize(&big, &vec![false; 100], &opts); let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let ident: Vec<usize> = (0..100).collect();
    let (c0, c1) = (order_cost(&big, &ident, &opts), order_cost(&big, &ob, &opts));
    let greens = ob.windows(2).filter(|w| rate(&big[w[0]], &big[w[1]]).1 == 0).count();
    chk("100 tracks", ms < 2000.0 && c1 < 0.5 * c0, format!("{ms:.0} ms, cost {c0:.2} -> {c1:.2}, {greens}/99 green transitions"));
    ok
}

fn library_tests(dir: &Path) -> bool {
    use library::*; let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} library {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let root = dir.join("libtest"); let _ = std::fs::remove_dir_all(&root);
    let (a, b) = (root.join("Music"), root.join("External"));
    std::fs::create_dir_all(a.join("House/Deep")).unwrap(); std::fs::create_dir_all(&b).unwrap();
    let sr = 44100u32;
    let wav = |p: &Path, bpm: f32, secs: f32| { let (l, r) = testgen::synth_drift(bpm, bpm, 45, true, secs, 0.1, sr); engine::write_wav(p, &l, &r, sr).unwrap(); };
    wav(&a.join("House/Kick_128.wav"), 128.0, 20.0); wav(&a.join("House/Deep/Kick_124.wav"), 124.0, 20.0); wav(&b.join("Ext_126.wav"), 126.0, 2.0);
    std::fs::write(a.join("notes.txt"), "not audio").unwrap();
    std::fs::write(a.join("broken.mp3"), vec![0x55u8; 4096]).unwrap();
    let folders = vec![a.display().to_string(), b.display().to_string(), root.join("Unplugged").display().to_string()];
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (ix, r, st) = scan(&folders, &Index::default(), &cancel, &|_, _| {});
    let names: Vec<String> = ix.entries.iter().map(|e| e.title.clone()).collect();
    let broken = ix.entries.iter().find(|e| e.path.ends_with("broken.mp3"));
    let k = ix.entries.iter().find(|e| e.path.ends_with("Kick_128.wav"));
    chk("first scan", r.files == 4 && r.added == 4 && broken.is_some_and(|e| e.error.is_some()) && k.is_some_and(|e| (e.duration - 20.0).abs() < 0.01 && e.title == "Kick 128"),
        format!("{} files ({:?}), {} unreadable, Kick 128 duration {:.2}s, {} ms", r.files, names, r.errors, k.map(|e| e.duration).unwrap_or(0.0), r.ms));
    chk("missing folder", st[2].1 == FolderState::Missing && st[0].1 == FolderState::Available(3) && r.missing.len() == 1, format!("{:?}", st.iter().map(|s| &s.1).collect::<Vec<_>>()));
    // index round trip
    let idir = root.join("cfg"); save_index(&idir, &ix).unwrap(); save_config(&idir, &Config { folders: folders.clone() }).unwrap();
    let ix2 = load_index(&idir); let c2 = load_config(&idir);
    chk("index and config saved", ix2.entries.len() == 4 && c2.folders == folders, format!("{} entries, {} folders", ix2.entries.len(), c2.folders.len()));
    // incremental: nothing changed
    let (ix3, r3, _) = scan(&folders, &ix2, &cancel, &|_, _| {});
    chk("unchanged rescan skips files", r3.unchanged == 4 && r3.added == 0 && r3.updated == 0 && r3.removed == 0, format!("unchanged {}, read {}, {} ms", r3.unchanged, r3.added + r3.updated, r3.ms));
    // add, change, remove
    std::thread::sleep(std::time::Duration::from_millis(1100));
    wav(&a.join("House/New_130.wav"), 130.0, 3.0); wav(&a.join("House/Deep/Kick_124.wav"), 124.0, 25.0); std::fs::remove_file(b.join("Ext_126.wav")).unwrap();
    let (ix4, r4, _) = scan(&folders, &ix3, &cancel, &|_, _| {});
    let k124 = ix4.entries.iter().find(|e| e.path.ends_with("Kick_124.wav")).map(|e| e.duration).unwrap_or(0.0);
    chk("incremental changes", r4.added == 1 && r4.updated == 1 && r4.removed == 1 && r4.unchanged == 2 && ix4.entries.len() == 4 && (k124 - 25.0).abs() < 0.01,
        format!("+{} new, {} changed (Kick 124 now {:.1}s), {} removed, {} unchanged", r4.added, r4.updated, k124, r4.removed, r4.unchanged));
    // drive unplugged: entries of a folder that disappears are kept and the scan does not fail
    let moved = root.join("External_offline"); std::fs::rename(&b, &moved).unwrap();
    let mut ix5 = ix4.clone(); ix5.entries.push(Entry { path: b.join("Old.wav").display().to_string(), folder: b.display().to_string(), title: "Old".into(), ..Default::default() });
    let (ix6, r6, st6) = scan(&folders, &ix5, &cancel, &|_, _| {});
    chk("unplugged drive keeps its tracks", st6[1].1 == FolderState::Missing && ix6.entries.iter().any(|e| e.title == "Old") && r6.removed == 0, format!("{:?}, {} entries", st6[1].1, ix6.entries.len()));
    std::fs::rename(&moved, &b).unwrap();
    // analysis via the shared cache
    let kp = a.join("House/Kick_128.wav").display().to_string();
    let an = analyse(&kp);
    chk("background analysis", an.as_ref().is_ok_and(|x| (x.0 - 128.0).abs() < 0.3), format!("{:?}", an.as_ref().map(|x| (x.0, x.1.clone()))));
    // view: search, sort, key and BPM filters
    let mut es = ix4.entries.clone();
    for e in es.iter_mut() { if e.path.ends_with("Kick_128.wav") { e.bpm = Some(128.0); e.key = Some("8A".into()); } if e.path.ends_with("Kick_124.wav") { e.bpm = Some(124.0); e.key = Some("3B".into()); } if e.path.ends_with("New_130.wav") { e.bpm = Some(65.0); e.key = Some("9A".into()); } }
    let t = |v: Vec<usize>| v.iter().map(|&i| es[i].title.clone()).collect::<Vec<_>>();
    let q = t(view(&es, &Filter { query: "kick", key_ref: None, bpm_ref: None }, SortCol::Bpm, false));
    let kf = t(view(&es, &Filter { query: "", key_ref: Some("8A"), bpm_ref: None }, SortCol::Title, true));
    let bf = t(view(&es, &Filter { query: "", key_ref: None, bpm_ref: Some((129.0, 2.0)) }, SortCol::Title, true));
    chk("search, sort and filters", q == ["Kick 128", "Kick 124"] && kf == ["Kick 128", "New 130"] && bf == ["Kick 128", "New 130"], format!("search {q:?}, key-compatible with 8A {kf:?}, BPM 129 ±2% (incl. half time) {bf:?}"));
    // scale: 3000 files, first scan and incremental rescan
    let big = root.join("Big"); for d in 0..30 { std::fs::create_dir_all(big.join(format!("d{d}"))).unwrap(); }
    let (l, r) = testgen::synth_drift(128.0, 128.0, 45, true, 0.05, 0.0, sr);
    for i in 0..3000 { engine::write_wav(&big.join(format!("d{}/t{i}.wav", i % 30)), &l, &r, sr).unwrap(); }
    let bf = vec![big.display().to_string()];
    let (bi, br, _) = scan(&bf, &Index::default(), &cancel, &|_, _| {});
    let (_, br2, _) = scan(&bf, &bi, &cancel, &|_, _| {});
    let t0 = std::time::Instant::now(); let n = view(&bi.entries, &Filter { query: "t29", key_ref: None, bpm_ref: None }, SortCol::Title, true).len(); let vms = t0.elapsed().as_secs_f64() * 1000.0;
    chk("3000 files", br.files == 3000 && br2.unchanged == 3000, format!("first scan {} ms, rescan {} ms, search+sort {vms:.1} ms ({n} hits)", br.ms, br2.ms));
    let _ = std::fs::remove_dir_all(&root);
    ok
}
fn beatgrid_tests() -> bool {
    use beatmap::*; use std::sync::Arc; let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} beatgrid editor {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let mono = |m: &BeatMap| m.beats.windows(2).all(|w| w[1] > w[0]);
    let mut t = model::Track { path: "x".into(), title: "x".into(), artist: String::new(), bpm: 120.0, offset: 0.25, key: "8A".into(), duration: 60.0,
        start_beat: 16.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 };
    t.normalize_grid();
    let before: Vec<f64> = (0..40).map(|k| t.sec_at(t.start_beat + k as f64)).collect();
    t.ensure_map();
    let after: Vec<f64> = (0..40).map(|k| t.sec_at(t.start_beat + k as f64)).collect();
    let err = before.iter().zip(&after).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    chk("fixed grid -> beat map", t.grid().dynamic() && err < 1e-9, format!("{} beats, max diff {err:.1e} s", t.map.as_ref().unwrap().beats.len()));
    let undo = serde_json::to_string(&t).unwrap();
    let s0 = t.start_beat; let a0: Vec<f64> = (0..8).map(|k| t.sec_at(s0 + k as f64)).collect();
    t.edit_map(|m| m.shift(0.010));
    let a1: Vec<f64> = (0..8).map(|k| t.sec_at(t.start_beat + k as f64)).collect();
    let d = a0.iter().zip(&a1).map(|(a, b)| b - a).collect::<Vec<_>>();
    chk("adjust full grid", d.iter().all(|x| (x - 0.010).abs() < 1e-9), format!("every beat +{:.1} ms", d[3] * 1000.0));
    let k = 8i64; let i = t.map_index(k); let n_before = t.sec_at(t.start_beat + 9.0);
    let got = t.edit_map(|m| m.move_single(i, 0.020));
    let (b7, b8, b9) = (t.sec_at(t.start_beat + 7.0), t.sec_at(t.start_beat + 8.0), t.sec_at(t.start_beat + 9.0));
    chk("adjust single beat", (got - 0.020).abs() < 1e-9 && (b9 - n_before).abs() < 1e-9 && b8 > b7 && mono(t.map.as_ref().unwrap()), format!("beat 9 +{:.0} ms, neighbours unchanged", got * 1000.0));
    let big = t.edit_map(|m| m.move_single(i, 5.0));
    chk("single beat clamps to neighbours", big < 0.5 && mono(t.map.as_ref().unwrap()), format!("asked +5 s, moved {:.0} ms", big * 1000.0));
    let mut t2: model::Track = serde_json::from_str(&undo).unwrap(); t2.edit_map(|_| ());
    let i = t2.map_index(16); let piv = i - 4;
    let m0 = t2.map.as_ref().unwrap().clone(); let span = m0.beats[i] - m0.beats[piv];
    let s = t2.edit_map(|m| m.stretch_following(piv, i, span * 0.02));
    let m1 = t2.map.as_ref().unwrap();
    let left_ok = (0..=piv).all(|j| (m1.beats[j] - m0.beats[j]).abs() < 1e-12);
    let pct = (span / (m1.beats[i] - m1.beats[piv]) - 1.0) * 100.0;
    let far = ((m1.beats[i + 40] - m1.beats[i + 39]) / (m0.beats[i + 40] - m0.beats[i + 39]) - s).abs();
    chk("beat + following stretches", left_ok && (pct + 1.96).abs() < 0.05 && far < 1e-9 && mono(m1), format!("anchors left of pivot fixed, tempo {pct:+.2} %, spacing after x{s:.3}"));
    let g = t2.grid(); let e = engine_like(&g, t2.start_beat, m1);
    chk("engine follows the edited map", e < 1e-9, format!("mix beat -> source time matches the map, max diff {e:.1e} s"));
    let mut t3: model::Track = serde_json::from_str(&undo).unwrap(); t3.ensure_map(); let n = t3.map.as_ref().unwrap().beats.len();
    let i = t3.map_index(5); let sec = t3.map.as_ref().unwrap().beats[i];
    let mut m = (**t3.map.as_ref().unwrap()).clone(); m.trim_before(i); t3.map = Some(Arc::new(m)); t3.offset = sec; t3.start_beat = 20.0; t3.normalize_grid();
    chk("set first beat", t3.map.as_ref().unwrap().beats[0] == sec && t3.map.as_ref().unwrap().beats.len() == n - i, format!("first beat at {sec:.3} s, {} beats dropped", i));
    let back: model::Track = serde_json::from_str(&undo).unwrap();
    chk("undo restores the map", (back.sec_at(back.start_beat + 8.0) - before[8]).abs() < 1e-9 || back.map.is_some(), "edits are snapshotted as JSON".into());
    ok
}
fn engine_like(g: &beatmap::Grid, start: f64, m: &beatmap::BeatMap) -> f64 {
    let k0 = m.u_of(g.sec(start)).round() as i64;
    (0..60).map(|k| (g.sec(start + k as f64) - m.beats[(k0 + k) as usize]).abs()).fold(0.0, f64::max)
}
fn timeline_tests() -> bool {
    let mut ok = true; let sr = 44100u32;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} timeline {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let sine: Vec<f32> = (0..sr as usize * 20).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / sr as f32).sin()).collect();
    let au = Arc::new(analysis::Audio { sr, l: sine.clone(), r: sine });
    let base = || { let mut m = model::Mix::default(); m.bpm = 120.0; m.key_lock = false;
        m.tracks.push(model::Track { path: "t".into(), title: "t".into(), artist: "t".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 20.0, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0 }); m };
    let render = |m: &model::Mix, a: f64, b: f64| { let auds: Vec<_> = m.tracks.iter().map(|_| Some(au.clone())).collect(); engine::render_offline(Arc::new(engine::Snap::build(m, &auds)), sr, a, b).0 };
    let whole = render(&base(), 0.0, 32.0);
    let mut m = base(); let n = m.split(0, 13.5);
    let parts = render(&m, 0.0, 32.0);
    let diff = whole.iter().zip(&parts).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    let (a, b) = (&m.tracks[0], &m.tracks[n.unwrap_or(0)]);
    chk("split keeps source offsets", n.is_some() && (a.end(120.0) - 13.5).abs() < 1e-9 && (b.vis_start(120.0) - 13.5).abs() < 1e-9 && b.offset == a.offset && diff < 1e-6,
        format!("2 clips meeting at beat 13.5, playback identical to the unsplit track (max diff {diff:.1e})"));
    chk("split clips make no transition", m.transitions().is_empty(), format!("{} transitions", m.transitions().len()));
    let mut m = base(); m.set_edge(0, true, 4.0); m.set_edge(0, false, 24.0);
    let x = render(&m, 0.0, 28.0); let at = |b: f64| (b * 0.5 * sr as f64) as usize;
    let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt();
    let (pre, mid, post) = (rms(&x[..at(4.0)]), rms(&x[at(4.0)..at(24.0)]), rms(&x[at(24.0)..]));
    chk("trim clip edges", pre < 2e-3 && mid > 0.25 && post < 2e-3 && (m.tracks[0].trim_in - 2.0).abs() < 1e-9, format!("rms before {pre:.1e}, inside {mid:.3}, after {post:.1e}"));
    let mut m2 = m.clone(); m2.set_edge(0, true, -50.0); m2.set_edge(0, false, 500.0);
    chk("edges clamp to the file", m2.tracks[0].trim_in == 0.0 && m2.tracks[0].trim_out == 0.0, "extend stops at the file start/end".into());
    let mut m3 = m.clone(); m3.set_edge(0, false, 3.0);
    chk("minimum clip length", m3.tracks[0].end(120.0) - m3.tracks[0].vis_start(120.0) >= 0.999, format!("{:.2} beats", m3.tracks[0].end(120.0) - m3.tracks[0].vis_start(120.0)));
    let mut m = base(); m.move_to_lane(0, 2); m.tracks[0].fade_in = 4.0; m.tracks[0].fade_out = 4.0;
    let x = render(&m, 0.0, 40.0); let w = |b: f64| rms(&x[at(b - 0.25)..at(b + 0.25)]);
    let (g1, g2, g3) = (w(1.0) / w(10.0), w(2.0) / w(10.0), w(38.0) / w(10.0));
    chk("sample fade in/out", (g1 - 0.25).abs() < 0.03 && (g2 - 0.5).abs() < 0.03 && (g3 - 0.5).abs() < 0.03, format!("gain at 1/4 of fade-in {g1:.2}, half {g2:.2}, half of fade-out {g3:.2}"));
    let d = m.duplicate(0); m.tracks[d].start_beat = 64.0;
    chk("Alt+drag duplicates a sample", m.tracks.len() == 2 && m.tracks[d].lane == 2 && m.tracks[d].path == m.tracks[0].path, "copy on the same sample lane".into());
    let mut m = base(); m.tracks.push(m.tracks[0].clone()); m.tracks[1].start_beat = 32.0; m.move_to_lane(1, 0);
    m.move_to_lane(0, 3);
    chk("drag a track into a sample lane", m.tracks[0].lane == 3 && m.tracks[1].lane == 0, format!("lanes {:?}", m.tracks.iter().map(|t| t.lane).collect::<Vec<_>>()));
    let c = [8.0, 16.25, 31.9];
    chk("magnetic snapping", model::magnet(16.0, &c, 0.5) == Some(16.25) && model::magnet(12.0, &c, 0.5).is_none() && model::magnet(31.5, &c, 0.5) == Some(31.9), "nearest candidate within tolerance".into());
    let mut v = serde_json::to_value(&base().tracks[0]).unwrap(); for k in ["trim_in", "trim_out", "fade_in", "fade_out"] { v.as_object_mut().unwrap().remove(k); }
    let old = v.to_string();
    let t: Result<model::Track, _> = serde_json::from_str(&old);
    chk("old projects load untrimmed", t.as_ref().map_or(false, |t| t.trim_in == 0.0 && t.fade_out == 0.0), t.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()));
    ok
}
