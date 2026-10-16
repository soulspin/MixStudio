#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod analysis;
mod automix;
mod library;
mod scratch;
mod phrase;
mod import;
mod export;
mod fx;
mod dialogs;
mod beatmap;
mod beatcheck;
mod gridtools;
mod app;
mod audio;
mod engine;
mod stems;
mod model;
mod sections;
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
        mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "Test".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] });
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
    ok &= stem_edit_tests();
    ok &= fx_tests();
    ok &= tempo_export_tests(dir);
    ok &= scratch_tests();
    ok &= phrase_tests();
    ok &= import_tests();
    ok &= ratio_tests();
    ok &= grid_mode_tests();
    println!("{}", if ok { "ALL TESTS PASSED" } else { "SOME TEST FAILED" });
    ok
}

fn one_track_mix(a: analysis::Audio, bpm: f64, mix_bpm: f64, key_lock: bool) -> (Arc<engine::Snap>, f64) {
    let dur = a.secs();
    let mut mix = model::Mix::default();
    mix.tracks.push(model::Track { path: "t".into(), title: "t".into(), artist: "t".into(), bpm, offset: 0.0, key: "8A".into(), duration: dur, start_beat: 0.0, lane: 0, trans: Default::default(), map: Option::None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] });
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
        for (k, lane) in [(0usize, 0usize), (1, 1)] { mix.tracks.push(model::Track { path: format!("{k}"), title: "t".into(), artist: "t".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 40.0, start_beat: if k == 0 { 0.0 } else { 40.0 }, lane, trans: Default::default(), map: Option::None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] }); }
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
            mix.add_track(model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "t".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] });
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
    model::Track { path: p.display().to_string(), title: an.title.clone(), artist: "t".into(), bpm: an.bpm, offset: an.offset, key: an.key.clone(), duration: an.duration, start_beat: 0.0, lane: 0, trans: Default::default(), map: an.beat_map(), locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] }
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
    let idir = root.join("cfg"); save_index(&idir, &ix).unwrap(); save_config(&idir, &Config { folders: folders.clone(), ..Default::default() }).unwrap();
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
        start_beat: 16.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
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
        m.tracks.push(model::Track { path: "t".into(), title: "t".into(), artist: "t".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 20.0, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] }); m };
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
fn stem_edit_tests() -> bool {
    let mut ok = true; let sr = 44100u32;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} stems {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let n = sr as usize * 20; let w = |f: f32, i: usize| 0.2 * (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32).sin();
    let (fd, fb, fm, fv) = (60.0f32, 110.0, 440.0, 880.0);
    let q = |f: f32| -> Vec<i16> { (0..n).map(|i| (w(f, i) * 32767.0) as i16).collect() };
    let mixv: Vec<f32> = (0..n).map(|i| w(fd, i) + w(fb, i) + w(fm, i) + w(fv, i)).collect();
    let au = Arc::new(analysis::Audio { sr, l: mixv.clone(), r: mixv });
    let st = Arc::new(stems::Stems { d: [q(fd), q(fd)], b: [q(fb), q(fb)], v: [q(fv), q(fv)], peaks: vec![] });
    let tone = |x: &[f32], f: f32, srate: f32| { let (mut re, mut im) = (0.0f64, 0.0f64); for (i, v) in x.iter().enumerate() { let p = 2.0 * std::f64::consts::PI * f as f64 * i as f64 / srate as f64; re += *v as f64 * p.cos(); im += *v as f64 * p.sin(); } (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32 };
    let mk = |mutes: &[(Vec<u8>, f64, f64, bool)], bpm: f64, kl: bool| {
        let mut m = model::Mix::default(); m.bpm = bpm; m.key_lock = kl; m.decks[0].vol = 1.0;
        let mut t = model::Track { path: "s".into(), title: "s".into(), artist: "s".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 20.0, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 2, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
        for (k, a, b, on) in mutes { t.set_mute(k, *a, *b, *on); }
        m.tracks.push(t); m };
    let render = |m: &model::Mix, from: f64, to: f64| engine::render_offline(Arc::new(engine::Snap::build_with(m, &[Some(au.clone())], &[Some(st.clone())])), sr, from, to).0;
    let m = mk(&[(vec![3], 4.0, 8.0, true)], 120.0, false); let x = render(&m, 0.0, 40.0);
    let win = |x: &[f32], a: f64, b: f64, f: f32| tone(&x[(a * sr as f64) as usize..(b * sr as f64) as usize], f, sr as f32);
    let ref_v = win(&x, 10.0, 12.0, fv);
    let (vin, min_, vout) = (win(&x, 5.0, 7.0, fv), win(&x, 5.0, 7.0, fm), win(&x, 10.0, 12.0, fv));
    chk("mute a selection on one stem", vin < 0.01 * ref_v && min_ > 0.5 * ref_v && vout > 0.1, format!("vocals {:.4} inside the muted range vs {:.4} outside, melody {:.4} still playing", vin, vout, min_));
    let m = mk(&[(model::lane_stems(1, 0), 12.0, 14.0, true)], 120.0, false); let x = render(&m, 0.0, 40.0);
    let (a1, a2, a3, v1) = (win(&x, 12.5, 13.5, fd), win(&x, 12.5, 13.5, fb), win(&x, 12.5, 13.5, fm), win(&x, 12.5, 13.5, fv));
    chk("2-stem view: mute instrumental", a1.max(a2).max(a3) < 0.005 && v1 > 0.1, format!("drums/bass/melody {:.4}/{:.4}/{:.4}, vocals {:.3}", a1, a2, a3, v1));
    let m = mk(&[(vec![3], 4.0, 8.0, true), (vec![3], 5.0, 6.0, false)], 120.0, false);
    let r = &m.tracks[0].stem_mutes;
    let x = render(&m, 0.0, 40.0);
    chk("unmute part of a range", r.len() == 2 && r.iter().any(|z| z.a == 4.0 && z.b == 5.0) && r.iter().any(|z| z.a == 6.0 && z.b == 8.0) && win(&x, 5.2, 5.8, fv) > 0.1 && win(&x, 6.2, 7.8, fv) < 0.01 * ref_v,
        format!("{:?}", r.iter().map(|z| (z.a, z.b)).collect::<Vec<_>>()));
    let m = mk(&[(vec![3], 4.0, 8.0, true)], 126.0, true); let x = render(&m, 0.0, 40.0); let k = 120.0 / 126.0;
    let (vin, vout) = (win(&x, 5.0 * k, 7.0 * k, fv), win(&x, 10.0 * k, 12.0 * k, fv));
    chk("mutes follow key-lock stretching", vin < 0.03 * vout && vout > 0.1, format!("vocals {:.4} muted vs {:.3} at 126 BPM", vin, vout));
    let clip = stems::render_clip(&au, &st, &[0.0, 0.0, 0.0, 1.0], 4.0, 8.0);
    let err = clip.l.iter().enumerate().map(|(i, v)| (v - w(fv, i + 4 * sr as usize)).abs()).fold(0.0f32, f32::max);
    chk("copy a stem selection to a clip", clip.l.len() == 4 * sr as usize && err < 2e-4, format!("{:.1} s vocal clip, max error {err:.1e}", clip.l.len() as f64 / sr as f64));
    let src = mk(&[], 120.0, false).tracks[0].clone(); let ct = src.clip_track(4.0, 8.0, "c".into(), "c".into(), 2);
    let e = (0..16).map(|k| { let b = 8.0 + k as f64 * 0.5; (ct.sec_at(b) + 4.0 - src.sec_at(b)).abs() }).fold(0.0, f64::max);
    chk("stem clip plays in sync", ct.lane == 2 && e < 1e-9 && (ct.vis_start(120.0) - 8.0).abs() < 1e-9, format!("clip starts at mix beat {:.2}, timing error {e:.1e} s", ct.vis_start(120.0)));
    let mut v = serde_json::to_value(&src).unwrap(); for k in ["stem_view", "stem_mutes"] { v.as_object_mut().unwrap().remove(k); }
    let t: Result<model::Track, _> = serde_json::from_str(&v.to_string());
    chk("old projects load without stem edits", t.as_ref().map_or(false, |t| t.stem_view == 0 && t.stem_mutes.is_empty()), "ok".into());
    ok
}
fn fx_tests() -> bool {
    use fx::*; let mut ok = true; let sr = 44100u32;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} automation/fx {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let mk = |f: f32| -> Arc<analysis::Audio> { let v: Vec<f32> = (0..sr as usize * 30).map(|i| 0.4 * (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32).sin()).collect(); Arc::new(analysis::Audio { sr, l: v.clone(), r: v }) };
    let track = || model::Track { path: "a".into(), title: "a".into(), artist: "a".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 30.0, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
    let mix1 = |t: model::Track| { let mut m = model::Mix::default(); m.bpm = 120.0; m.key_lock = false; m.decks[0].vol = 1.0; m.tracks.push(t); m };
    let render = |m: &model::Mix, au: &[Arc<analysis::Audio>], a: f64, b: f64| { let auds: Vec<_> = au.iter().map(|x| Some(x.clone())).collect(); engine::render_offline(Arc::new(engine::Snap::build(m, &auds)), sr, a, b).0 };
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    let at = |b: f64| (b * 0.5 * sr as f64) as usize; let win = |x: &[f32], b: f64| rms(&x[at(b - 0.25)..at(b + 0.25)]);
    let p = [(0.0, 1.0f32), (16.0, 0.0)];
    chk("interpolation", model::interp(&p, 4.0) == Some(0.75) && model::interp(&p, -3.0) == Some(1.0) && model::interp(&p, 40.0) == Some(0.0) && model::interp(&[], 1.0).is_none(), "linear between points, held outside".into());
    let a440 = mk(440.0);
    let mut t = track(); t.auto_set(0, 16.0, 0.0); t.auto_set(0, 0.0, 1.0);
    let x = render(&mix1(t.clone()), &[a440.clone()], 0.0, 24.0); let full = win(&x, 1.0) / 0.9375 * 1.0;
    let (g4, g12, g20) = (win(&x, 4.0) / full, win(&x, 12.0) / full, win(&x, 20.0) / full);
    chk("volume automation", (g4 - 0.75).abs() < 0.04 && (g12 - 0.25).abs() < 0.04 && g20 < 0.01, format!("gain at beat 4 {g4:.2} (0.75), 12 {g12:.2} (0.25), 20 {g20:.3} (0)"));
    let a60 = mk(60.0); let mut t = track(); t.auto_set(1, 0.0, -1.0);
    let cut = win(&render(&mix1(t), &[a60.clone()], 0.0, 12.0), 8.0); let flat = win(&render(&mix1(track()), &[a60.clone()], 0.0, 12.0), 8.0);
    chk("low EQ automation", (cut / flat) < 0.35 && (cut / flat) > 0.15, format!("60 Hz at low = -100 %: {:.1} dB", 20.0 * (cut / flat).log10()));
    let a2k = mk(2000.0); let mut t = track(); t.auto_set(4, 0.0, 0.0); t.auto_set(4, 8.0, -1.0);
    let x = render(&mix1(t), &[a2k.clone()], 0.0, 16.0);
    chk("filter automation", win(&x, 2.0) > 0.2 && win(&x, 12.0) < 0.02, format!("2 kHz: {:.3} before, {:.4} with the low-pass closed", win(&x, 2.0), win(&x, 12.0)));
    let two = |curve: bool| { let mut m = model::Mix::default(); m.bpm = 120.0; m.key_lock = false; m.decks[0].vol = 1.0; m.decks[1].vol = 1.0;
        let a = track(); let mut b = track(); b.start_beat = 32.0; b.offset = 0.0; b.path = "b".into(); m.tracks.push(a); m.tracks.push(b);
        m.tracks[0].duration = 24.0; m.tracks[1].lane = 1; if curve { m.tracks[1].trans.curve_out = vec![(0.0, 1.0), (1.0, 1.0)]; } m };
    let x0 = render(&two(false), &[a440.clone(), mk(880.0)], 0.0, 48.0); let x1 = render(&two(true), &[a440.clone(), mk(880.0)], 0.0, 48.0);
    let tr = two(false).transitions(); let mid = tr.first().map(|t| (t.2 + t.3) / 2.0).unwrap_or(40.0);
    let tone = |x: &[f32], f: f32| { let (mut re, mut im) = (0.0f64, 0.0f64); for (i, v) in x.iter().enumerate() { let p = 2.0 * std::f64::consts::PI * f as f64 * i as f64 / sr as f64; re += *v as f64 * p.cos(); im += *v as f64 * p.sin(); } (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32 };
    let (o0, o1) = (tone(&x0[at(mid - 0.25)..at(mid + 0.25)], 440.0), tone(&x1[at(mid - 0.25)..at(mid + 0.25)], 440.0));
    chk("transition volume automation", !tr.is_empty() && (o1 / 0.4 - 1.0).abs() < 0.05 && (o0 / 0.4 - 0.707).abs() < 0.08, format!("outgoing at the middle: preset {:.2}, custom flat curve {:.2}", o0 / 0.4, o1 / 0.4));
    // effect blocks on a short burst (beats 0..1)
    let burst = { let mut t = track(); t.trim_out = 30.0 - 0.5; t };
    let with = |f: Option<FxBlock>| { let mut m = mix1(burst.clone()); if let Some(f) = f { m.fx.push(f); } render(&m, &[a440.clone()], 0.0, 24.0) };
    let dry = with(None);
    let mut rv = FxBlock::new(FxKind::Reverb, 0, 0.0); rv.len = 2.0; rv.tail = 6.0; rv.decay = 0.8;
    let wet = with(Some(rv.clone()));
    let (t3, t12) = (rms(&wet[at(3.0)..at(5.0)]), rms(&wet[at(12.0)..at(16.0)]));
    chk("reverb block with tail", rms(&dry[at(3.0)..at(5.0)]) < 1e-4 && t3 > 0.002 && t12 < 1e-4, format!("after the clip: tail {t3:.4}, after the tail {t12:.1e}"));
    let mut ec = FxBlock::new(FxKind::Echo, 0, 0.0); ec.len = 1.0; ec.time = 2.0; ec.tail = 8.0; ec.decay = 0.5;
    let e = with(Some(ec)); let (r1, r2, gap) = (rms(&e[at(2.05)..at(2.45)]), rms(&e[at(4.05)..at(4.45)]), rms(&e[at(1.2)..at(1.8)]));
    chk("echo block repeats on time", r1 > 0.05 && r2 > 0.02 && r2 < r1 && gap < 0.01, format!("repeats at +2 beats {r1:.3}, +4 beats {r2:.3}, silence between {gap:.4}"));
    let n = mix1(track()); let plain = render(&n, &[a440.clone()], 0.0, 16.0);
    let mut fl = FxBlock::new(FxKind::Flanger, 0, 4.0); fl.len = 4.0; let mut m = mix1(track()); m.fx.push(fl); let f = render(&m, &[a440.clone()], 0.0, 16.0);
    let dif = |a: f64, b: f64| f.iter().zip(&plain).skip(at(a)).take(at(b) - at(a)).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    chk("flanger block only inside its range", dif(5.0, 7.5) > 0.05 && dif(0.0, 3.9) < 1e-6 && dif(8.2, 15.0) < 1e-6, format!("max change inside {:.3}, outside {:.1e}", dif(5.0, 7.5), dif(8.2, 15.0)));
    let a5k = mk(5000.0); let mut fs = FxBlock::new(FxKind::FilterSweep, 4, 0.0); fs.len = 16.0; fs.depth = 1.0; let mut m = mix1(track()); m.fx.push(fs); let x = render(&m, &[a5k], 0.0, 20.0);
    chk("filter sweep block (on master)", win(&x, 1.0) > 0.2 && win(&x, 15.0) < 0.05 && win(&x, 19.0) > 0.2, format!("5 kHz: start {:.3}, end of sweep {:.4}, after {:.3}", win(&x, 1.0), win(&x, 15.0), win(&x, 19.0)));
    let mut s = engine::Snap::build(&mix1(burst.clone()), &[Some(a440.clone())]); let mut lv = FxBlock::new(FxKind::Reverb, 4, 0.0); lv.mix = 1.0; lv.decay = 0.9; s.live = Some(lv);
    let l = engine::render_offline(Arc::new(s), sr, 0.0, 8.0).0;
    chk("XY pad live effect", rms(&l[at(3.0)..at(5.0)]) > 0.003, format!("live reverb tail {:.4}", rms(&l[at(3.0)..at(5.0)])));
    let dir = std::env::temp_dir().join(format!("ms-fxp-{}", std::process::id())); std::env::set_var("MIXSTUDIO_CONFIG_DIR", &dir);
    let pr = vec![FxPreset { name: "Big room".into(), block: rv.clone() }]; let saved = save_presets(&pr).is_ok(); let back = load_presets();
    std::env::remove_var("MIXSTUDIO_CONFIG_DIR"); let _ = std::fs::remove_dir_all(&dir);
    chk("save a block as a preset", saved && back == pr, format!("{} preset(s) round-tripped", back.len()));
    let mut v = serde_json::to_value(&mix1(track())).unwrap(); v.as_object_mut().unwrap().remove("fx"); v["tracks"][0].as_object_mut().unwrap().remove("auto");
    let old: Result<model::Mix, _> = serde_json::from_value(v);
    chk("old projects load", old.as_ref().map_or(false, |m| m.fx.is_empty() && m.tracks[0].auto.is_empty()), old.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()));
    ok
}
fn tempo_export_tests(dir: &Path) -> bool {
    let mut ok = true; let sr = 44100u32;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} tempo/export {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let sine: Vec<f32> = (0..sr as usize * 45).map(|i| 0.4 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin()).collect();
    let au = Arc::new(analysis::Audio { sr, l: sine.clone(), r: sine });
    let track = |bpm: f64| model::Track { path: "a".into(), title: "a".into(), artist: "a".into(), bpm, offset: 0.0, key: "8A".into(), duration: 45.0, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
    let mut m = model::Mix::default(); m.bpm = 120.0; m.tracks.push(track(120.0)); m.tempo_mode = 3; m.ramp = (120.0, 130.0);
    let len = m.length_beats(); let c = m.tempo_curve();
    let exact = 60.0 * len / 10.0 * (130.0f64 / 120.0).ln();
    let snap = Arc::new(engine::Snap::build(&m, &[Some(au.clone())]));
    let x = engine::render_offline(snap.clone(), sr, 0.0, len).0;
    chk("fixed ramp: length follows the tempo curve", c.len() == 2 && ((x.len() as f64 / sr as f64) - exact).abs() < 0.01, format!("120 → 130 BPM over {len:.0} beats: {:.3} s (exact {exact:.3} s)", x.len() as f64 / sr as f64));
    let at = |m: &model::Mix, b: f64| (m.secs(0.0, b) * sr as f64) as usize;
    let fr = |x: &[f32], m: &model::Mix, b: f64| freq(&x[at(m, b - 1.0)..at(m, b + 1.0)], sr as f32);
    let target = 440.0 * m.bpm_at(len - 4.0) as f32 / 120.0;
    for (st, name, exp) in [(1u8, "key lock", 440.0f32), (2, "vinyl", target), (3, "beat slice", 440.0)] {
        let mut mm = m.clone(); mm.tracks[0].stretch = st;
        let x = engine::render_offline(Arc::new(engine::Snap::build(&mm, &[Some(au.clone())])), sr, 0.0, len).0;
        let f = fr(&x, &mm, len - 4.0);
        chk(&format!("stretch mode {name} on the ramp"), (f - exp).abs() < 4.0, format!("{f:.1} Hz near the end at {:.1} BPM (exp {exp:.1})", mm.bpm_at(len - 4.0)));
    }
    let mut mm = model::Mix::default(); mm.bpm = 100.0; mm.tracks.push(track(120.0)); mm.tracks[0].stretch = 3;
    let x = engine::render_offline(Arc::new(engine::Snap::build(&mm, &[Some(au.clone())])), sr, 0.0, 16.0).0;
    let beat = (0.6 * sr as f64) as usize; let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt();
    let (head, tail) = ((4..12).map(|k| rms(&x[k * beat..k * beat + beat * 7 / 10])).sum::<f32>() / 8.0, (4..12).map(|k| rms(&x[k * beat + beat * 87 / 100..(k + 1) * beat - 10])).sum::<f32>() / 8.0);
    chk("beat slice at a slower tempo", head > 0.2 && tail < 0.01 && (freq(&x[4 * beat..4 * beat + beat / 2], sr as f32) - 440.0).abs() < 4.0, format!("120-BPM track at 100 BPM: slices at the original pitch (rms {head:.3}), gap at the end of each beat (rms {tail:.4})"));
    let mut mm = model::Mix::default(); mm.bpm = 120.0; mm.tempo_mode = 2; mm.tempo_pts = vec![(0.0, 120.0), (32.0, 120.0), (40.0, 140.0)];
    chk("manual tempo nodes", (mm.bpm_at(16.0) - 120.0).abs() < 1e-9 && (mm.bpm_at(36.0) - 130.0).abs() < 1e-9 && (mm.bpm_at(60.0) - 140.0).abs() < 1e-9, "120 → 130 (between nodes) → 140 BPM".into());
    let mut ma = model::Mix::default(); ma.bpm = 120.0; ma.tempo_mode = 1; let a = track(120.0); let mut b = track(126.0); b.start_beat = 64.0; b.lane = 1; b.path = "b".into(); ma.tracks.push(a); ma.tracks.push(b);
    let tr = ma.transitions(); let (s, e) = tr.first().map(|t| (t.2, t.3)).unwrap_or((0.0, 0.0));
    chk("auto tempo glides through the transition", !tr.is_empty() && (ma.bpm_at(s - 8.0) - 120.0).abs() < 1e-9 && (ma.bpm_at((s + e) / 2.0) - 123.0).abs() < 1e-6 && (ma.bpm_at(e + 8.0) - 126.0).abs() < 1e-9,
        format!("120 BPM, {:.1} BPM in the middle of the transition, 126 BPM after", ma.bpm_at((s + e) / 2.0)));
    let tone: Vec<f32> = (0..sr as usize).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr as f32).sin()).collect();
    for (fmt, ext) in [(0u8, "wav"), (1, "wav"), (2, "flac"), (3, "mp3")] {
        let p = dir.join(format!("export-{fmt}.{ext}"));
        if fmt == 3 && !export::has_ffmpeg() { println!("SKIP tempo/export MP3: ffmpeg not installed"); continue; }
        let t0 = std::time::Instant::now(); let r = export::write(&p, &tone, &tone, sr, fmt);
        let back = r.clone().and_then(|_| analysis::decode(&p).map_err(|e| e.to_string()));
        let (pass, info) = match back {
            Ok((a, _, _)) => { let n = a.l.len().min(tone.len()); let lag = if fmt == 3 { (0..3000).min_by_key(|&d| (a.l.iter().skip(d).zip(&tone).take(4000).map(|(x, y)| (x - y).abs()).sum::<f32>() * 1000.0) as i64).unwrap_or(0) } else { 0 };
                let err = a.l.iter().skip(lag).zip(&tone).take(n.saturating_sub(lag + 2000)).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                let lim = match fmt { 0 => 1e-4, 1 | 2 => 1e-6, _ => 0.05 };
                let bits = if fmt == 0 { 16 } else if fmt == 3 { 0 } else { 24 };
                (err < lim && a.sr == sr, format!("{} written in {} ms, decoded back: max error {err:.1e}{}", export::FORMATS[fmt as usize].0, t0.elapsed().as_millis(), if bits > 0 { format!(" ({bits}-bit)") } else { String::new() })) }
            Err(e) => (false, e),
        };
        chk(&format!("export {}", export::FORMATS[fmt as usize].0), pass, info);
    }
    let mut v = serde_json::to_value(&m).unwrap(); for k in ["tempo_mode", "tempo_pts", "ramp"] { v.as_object_mut().unwrap().remove(k); } v["tracks"][0].as_object_mut().unwrap().remove("stretch");
    let old: Result<model::Mix, _> = serde_json::from_value(v);
    chk("old projects keep a constant tempo", old.as_ref().map_or(false, |m| m.tempo_mode == 0 && m.tempo_curve().is_empty() && m.tracks[0].stretch == 0), "ok".into());
    ok
}
fn scratch_tests() -> bool {
    use fx::*; let mut ok = true; let sr = 44100u32;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} scratch {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let n = sr as usize * 30;
    let ramp: Vec<f32> = (0..n).map(|i| 0.7 * i as f32 / n as f32).collect();
    let ra = Arc::new(analysis::Audio { sr, l: ramp.clone(), r: ramp });
    let sine: Vec<f32> = (0..n).map(|i| 0.4 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin()).collect();
    let sa = Arc::new(analysis::Audio { sr, l: sine.clone(), r: sine });
    let mk = |lane: usize, pat: &str, len: f64| { let mut m = model::Mix::default(); m.bpm = 120.0; m.decks[lane].vol = 1.0;
        m.tracks.push(model::Track { path: "a".into(), title: "a".into(), artist: "a".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 30.0, start_beat: 0.0, lane, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 2, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] });
        let mut b = FxBlock::new(FxKind::Scratch, lane, 8.0); b.pattern = pat.into(); b.len = len; b.ease = 0.0; m.fx.push(b); m };
    let render = |m: &model::Mix, a: &Arc<analysis::Audio>| engine::render_offline(Arc::new(engine::Snap::build(m, &[Some(a.clone())])), sr, 0.0, 16.0).0;
    let p = scratch::parse("EA.").unwrap();
    let (x0, x5, x9) = (scratch::eval(&p, 0.0, 0.0).0, scratch::eval(&p, 0.5, 0.0).0, scratch::eval(&p, 0.9, 0.0).0);
    chk("pattern syntax", scratch::parse("AEx.").is_err() && scratch::parse(">r>r.").map(|p| p[0].len()) == Ok(4) && (x0 - 1.0).abs() < 1e-9 && (x5 - 0.5).abs() < 1e-9 && (x9 - 0.1).abs() < 1e-9,
        format!("EA.: position {x0:.2} → {x5:.2} → {x9:.2} (reverse); bad characters rejected"));
    let all_ok = scratch::PRESETS.iter().all(|(_, s)| scratch::parse(s).is_ok());
    chk("presets parse", all_ok, scratch::PRESETS.iter().map(|p| format!("{} '{}'", p.0, p.1)).collect::<Vec<_>>().join(", "));
    let src = |sec: f64| 0.7 * (sec * sr as f64) as f32 / n as f32;
    let m = mk(0, "EA.", 1.0); let x = render(&m, &ra); let y = render(&m, &ra);
    chk("deterministic render", x == y, format!("{} samples identical", x.len()));
    let at = |b: f64| x[(b * 0.5 * sr as f64) as usize];
    let errs: Vec<f32> = [0.1, 0.3, 0.6, 0.9].iter().map(|u| (at(8.0 + u) - src((8.0 + (1.0 - u)) * 0.5)).abs()).collect();
    let mono = (1..40).all(|k| at(8.0 + k as f64 * 0.024) < at(8.0 + (k - 1) as f64 * 0.024));
    chk("reverse plays the beat backwards", mono && errs.iter().all(|e| *e < 2e-3) && (at(7.5) - src(3.75)).abs() < 2e-3 && (at(9.5) - src(4.75)).abs() < 2e-3,
        format!("position falls through the beat (max error {:.1e}), normal playback before and after", errs.iter().fold(0.0f32, |a, b| a.max(*b))));
    let m = mk(0, "EA.", 4.0); let x = render(&m, &ra); let at = |b: f64| x[(b * 0.5 * sr as f64) as usize];
    let e: f32 = (0..4).map(|k| (at(8.0 + k as f64 + 0.25) - src((8.0 + k as f64 + 0.75) * 0.5)).abs()).fold(0.0, f32::max);
    chk("pattern loops over a longer block", e < 2e-3, format!("each of the 4 beats reversed, max error {e:.1e}"));
    let m = mk(2, "EA.", 1.0); let x = render(&m, &ra); let at = |b: f64| x[(b * 0.5 * sr as f64) as usize];
    chk("works on sample lanes", (at(8.25) - src(8.75 * 0.5)).abs() < 2e-3, "reverse on Sample 1".into());
    let m = mk(0, ">r>r.", 4.0); let x = render(&m, &sa);
    let rms = |a: f64, b: f64| { let s = &x[(a * 0.5 * sr as f64) as usize..(b * 0.5 * sr as f64) as usize]; (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt() };
    let (on, off): (f32, f32) = ((0..4).map(|k| rms(8.0 + k as f64 + 0.03, 8.0 + k as f64 + 0.22)).sum::<f32>() / 4.0, (0..4).map(|k| rms(8.0 + k as f64 + 0.28, 8.0 + k as f64 + 0.47)).sum::<f32>() / 4.0);
    chk("transformer gating mutes", on > 0.25 && off < 1e-3, format!("open slots rms {on:.3}, released slots {off:.1e}"));
    let mut v = serde_json::to_value(FxBlock::new(FxKind::Reverb, 0, 0.0)).unwrap(); for k in ["pattern", "ease"] { v.as_object_mut().unwrap().remove(k); }
    chk("old effect blocks load", serde_json::from_value::<FxBlock>(v).is_ok(), "ok".into());
    ok
}

fn phrase_tests() -> bool {
    use crate::phrase;
    let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} phrase {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let sr = 22050u32;
    let names = |v: &[(f64, u8)]| v.iter().map(|x| format!("{}:{}", x.0 / 4.0, phrase::LABELS[x.1 as usize])).collect::<Vec<_>>().join(" ");
    let want: Vec<(f64, u8)> = vec![(0.0, 0), (64.0, 1), (128.0, 2), (256.0, 3), (320.0, 2), (448.0, 4)];
    for (bpm, li) in [(128.0f32, 0.0f32), (124.0, 1.3), (140.0, 0.5)] {
        let (l, r) = crate::testgen::synth_structured(bpm, li, sr); let a = analysis::Audio { sr, l, r };
        let p = analysis::peaks(&a); let f = phrase::bar_features(&p, sr, analysis::PEAK_HOP, bpm as f64, li as f64);
        let d = phrase::detect(&f);
        chk(&format!("synthetic structure at {bpm} BPM"), d == want, format!("bars {}", names(&d)));
    }
    // full analysis path (beat + downbeat detection) at 44.1 kHz
    { let (l, r) = crate::testgen::synth_structured(126.0, 0.0, 44100); let a = analysis::Audio { sr: 44100, l, r };
      let an = analysis::analyse(&a, "s".into(), "s".into(), Path::new("s.wav"));
      let bar = |b: f64| (b + an.offset * an.bpm / 60.0) / 4.0;
      let drops: Vec<f64> = an.sections.iter().filter(|x| x.1 == 2).map(|x| bar(x.0)).collect();
      let pass = an.version == analysis::ANALYSIS_VERSION && drops.len() == 2 && (drops[0] - 32.0).abs() <= 1.0 && (drops[1] - 80.0).abs() <= 1.0 && an.sections.first().map(|x| x.1) == Some(0);
      chk("detected in the analysis", pass, format!("BPM {:.1}, cache v{}, phrases {}", an.bpm, an.version, names(&an.sections))); }
    chk("cache version bumped", analysis::ANALYSIS_VERSION >= 7, format!("v{}", analysis::ANALYSIS_VERSION));
    // editable labels, persisted in the project
    let mut m = model::Mix::default();
    let det = want.clone(); let mut ed = det.clone(); ed[3].1 = 1; m.phrase_edit.insert("x.wav".into(), ed.clone());
    let j = serde_json::to_string(&m).unwrap(); let m2: model::Mix = serde_json::from_str(&j).unwrap();
    let old: Result<model::Mix, _> = serde_json::from_str(&j.replace(&format!(",\"phrase_edit\":{}", serde_json::to_string(&m.phrase_edit).unwrap()), ""));
    chk("labels editable and saved", m2.phrases("x.wav", &det)[3].1 == 1 && m2.phrases("y.wav", &det)[3].1 == 3 && old.is_ok(), "override kept, other files use detection, old projects load".into());
    // auto-mix: next intro starts on the previous outro
    let tr = |p: &str| model::Track { path: p.into(), title: "a".into(), artist: "a".into(), bpm: 120.0, offset: 0.0, key: "8A".into(), duration: 260.0, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
    let mut m = model::Mix::default(); m.bpm = 120.0; m.tracks = vec![tr("a"), tr("b"), tr("c")];
    let ph = vec![want.clone(), want.clone(), vec![(0.0, 2)]];
    m.relayout_phrases(&[0, 1, 2], 16.0, &ph);
    let (s0, s1, s2) = (m.tracks[0].start_beat, m.tracks[1].start_beat, m.tracks[2].start_beat);
    let e1 = m.tracks[1].end(120.0);
    chk("auto-mix overlaps outro with intro", s1 - s0 == 448.0 && ((s2 - (e1 - 64.0)) / 4.0).round() == (s2 - (e1 - 64.0)) / 4.0 && (e1 - s2 - 64.0).abs() < 4.0, format!("B starts at A's outro (beat {s1}), C (no intro) falls back to 16 bars"));
    ok
}

fn import_tests_core() -> bool {
    use crate::import::{self, Remap, Source};
    let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} import {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let dir = std::env::temp_dir().join(format!("mixstudio-import-test-{}", std::process::id())); let _ = std::fs::remove_dir_all(&dir);
    let music = dir.join("music"); std::fs::create_dir_all(music.join("sub dir")).unwrap();
    for f in ["a.mp3", "b.flac", "sub dir/c & d.wav"] { std::fs::write(music.join(f), b"x").unwrap(); }
    let m = music.display().to_string();
    let fp = |p: &std::path::Path| { use std::hash::{Hash, Hasher}; let b = std::fs::read(p).unwrap_or_default(); let mut h = std::collections::hash_map::DefaultHasher::new(); b.hash(&mut h);
        (h.finish(), std::fs::metadata(p).and_then(|m| m.modified()).ok()) };
    let names = |n: &import::Node| n.children.iter().map(|c| format!("{}{}", c.name, if c.folder { "/" } else { "" })).collect::<Vec<_>>().join(", ");
    // Rekordbox (Windows paths)
    let rb = dir.join("rekordbox.xml");
    std::fs::write(&rb, r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0"><PRODUCT Name="rekordbox" Version="6.8.5"/>
<COLLECTION Entries="3">
<TRACK TrackID="11" Name="Alpha &amp; Omega" Artist="DJ A" Album="X" Genre="House" TotalTime="300" AverageBpm="124.00" Tonality="Am" Location="file://localhost/C:/Users/Mike/Music/a.mp3">
 <TEMPO Inizio="0.025" Bpm="124.00" Metro="4/4" Battito="1"/>
 <POSITION_MARK Name="Drop" Type="0" Start="64.512" Num="0"/><POSITION_MARK Name="" Type="0" Start="12.000" Num="-1"/></TRACK>
<TRACK TrackID="12" Name="Beta" Artist="DJ B" AverageBpm="70.00" Tonality="8B" Location="file://localhost/C:/Users/Mike/Music/b.flac"/>
<TRACK TrackID="13" Name="Gamma" Artist="DJ C" AverageBpm="128.00" Tonality="" Location="file://localhost/C:/Users/Mike/Music/sub%20dir/c%20&amp;%20d.wav"/>
</COLLECTION>
<PLAYLISTS><NODE Type="0" Name="ROOT" Count="2">
 <NODE Type="0" Name="Club" Count="1"><NODE Name="Peak" Type="1" KeyType="0" Entries="2"><TRACK Key="12"/><TRACK Key="11"/></NODE></NODE>
 <NODE Name="Warmup" Type="1" KeyType="1" Entries="1"><TRACK Key="file://localhost/C:/Users/Mike/Music/sub%20dir/c%20&amp;%20d.wav"/></NODE>
</NODE></PLAYLISTS></DJ_PLAYLISTS>"#).unwrap();
    let maps = vec![Remap { from: "C:\\Users\\Mike\\Music".into(), to: m.clone() }];
    let before = fp(&rb);
    match import::load(&rb, &maps) { Ok(l) => {
        let t = &l.tracks; let peak = l.root.at(&[0, 0]);
        let pass = l.source == Source::Rekordbox && t.len() == 3 && t[0].title == "Alpha & Omega" && t[0].bpm == Some(124.0) && t[0].key.as_deref() == Some("Am") && t[0].cues.len() == 2 && (t[0].cues[0].pos - 64.512).abs() < 1e-9 && t[0].cues[0].name == "Drop" && t[0].cues[0].hot == 0
            && t.iter().all(|x| x.exists) && t[2].path == format!("{m}/sub dir/c & d.wav") && t[2].key.is_none()
            && l.root.children.len() == 2 && l.root.children[0].folder && peak.is_some_and(|p| p.name == "Peak" && p.tracks == vec![1, 0]) && l.root.children[1].tracks == vec![2];
        chk("Rekordbox XML", pass, format!("{} tracks, BPM {:?}, key {:?}, {} cues, tree: {} > {}", t.len(), t[0].bpm, t[0].key, t[0].cues.len(), names(&l.root), l.root.children.first().map(names).unwrap_or_default())); }
        Err(e) => chk("Rekordbox XML", false, e) }
    // remapping rules
    let r1 = import::remap("C:\\Users\\Mike\\Music\\x\\y.mp3", &[Remap { from: "c:/users/mike/music/".into(), to: "/mnt/win/Users/Mike/Music".into() }, Remap { from: "C:/".into(), to: "/mnt/c".into() }]);
    let r2 = import::remap("C:/Other/z.mp3", &[Remap { from: "C:/".into(), to: "/mnt/c".into() }, Remap { from: "C:/Oth".into(), to: "/bad".into() }]);
    chk("path remapping", r1 == "/mnt/win/Users/Mike/Music/x/y.mp3" && r2 == "/mnt/c/Other/z.mp3" && import::remap("/home/a.mp3", &[]) == "/home/a.mp3", format!("{r1}, {r2}"));
    // Traktor (macOS style volume, nested folders)
    let nml = dir.join("collection.nml");
    let d = m.trim_start_matches('/').replace('/', "/:");
    std::fs::write(&nml, format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<NML VERSION="19"><HEAD COMPANY="www.native-instruments.com" PROGRAM="Traktor"/>
<COLLECTION ENTRIES="2">
<ENTRY TITLE="Alpha" ARTIST="DJ A"><LOCATION DIR="/:{d}/:" FILE="a.mp3" VOLUME="Macintosh HD" VOLUMEID="x"/><ALBUM TITLE="Alb"/>
 <INFO GENRE="Techno" PLAYTIME="301" PLAYTIME_FLOAT="300.5"/><TEMPO BPM="125.000061" BPM_QUALITY="100"/><MUSICAL_KEY VALUE="21"/>
 <CUE_V2 NAME="AutoGrid" DISPL_ORDER="0" TYPE="4" START="20.0" LEN="0" REPEATS="-1" HOTCUE="-1"/>
 <CUE_V2 NAME="Intro" DISPL_ORDER="0" TYPE="0" START="1500.0" LEN="0" REPEATS="-1" HOTCUE="1"/></ENTRY>
<ENTRY TITLE="Beta" ARTIST="DJ B"><LOCATION DIR="/:{d}/:" FILE="b.flac" VOLUME="Macintosh HD"/><INFO KEY="11m"/><TEMPO BPM="140"/></ENTRY>
</COLLECTION>
<PLAYLISTS><NODE TYPE="FOLDER" NAME="$ROOT"><SUBNODES COUNT="1">
 <NODE TYPE="FOLDER" NAME="Gigs"><SUBNODES COUNT="1"><NODE TYPE="PLAYLIST" NAME="Berlin"><PLAYLIST ENTRIES="2" TYPE="LIST" UUID="u">
  <ENTRY><PRIMARYKEY TYPE="TRACK" KEY="Macintosh HD/:{d}/:b.flac"/></ENTRY><ENTRY><PRIMARYKEY TYPE="TRACK" KEY="Macintosh HD/:{d}/:a.mp3"/></ENTRY></PLAYLIST></NODE></SUBNODES></NODE>
</SUBNODES></NODE></PLAYLISTS></NML>"#)).unwrap();
    match import::load(&nml, &[]) { Ok(l) => { let t = &l.tracks; let b = l.root.at(&[0, 0]);
        let pass = l.source == Source::Traktor && t.len() == 2 && t[0].path == format!("{m}/a.mp3") && t[0].exists && (t[0].bpm.unwrap_or(0.0) - 125.0).abs() < 0.01 && t[0].key.as_deref() == Some("8A")
            && t[0].cues == vec![import::Cue { name: "Intro".into(), pos: 1.5, hot: 1 }] && t[1].key.as_deref() == Some("11m") && (t[0].duration - 300.5).abs() < 1e-9 && b.is_some_and(|b| b.name == "Berlin" && b.tracks == vec![1, 0]) && l.root.children[0].folder;
        chk("Traktor collection.nml", pass, format!("{} tracks, BPM {:?}, keys {:?}/{:?}, cues {:?}, Gigs > Berlin", t.len(), t[0].bpm, t[0].key, t[1].key, t[0].cues.len())); }
        Err(e) => chk("Traktor collection.nml", false, e) }
    // iTunes / Music library XML with a playlist folder
    let it = dir.join("iTunes Music Library.xml");
    let url = format!("file://{}", m.replace(' ', "%20"));
    std::fs::write(&it, format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Major Version</key><integer>1</integer><key>Tracks</key><dict>
<key>101</key><dict><key>Track ID</key><integer>101</integer><key>Name</key><string>Alpha</string><key>Artist</key><string>DJ A</string><key>BPM</key><integer>124</integer><key>Total Time</key><integer>300000</integer><key>Location</key><string>{url}/a.mp3</string></dict>
<key>102</key><dict><key>Track ID</key><integer>102</integer><key>Name</key><string>C &amp; D</string><key>Total Time</key><integer>1000</integer><key>Location</key><string>{url}/sub%20dir/c%20&amp;%20d.wav</string></dict>
</dict><key>Playlists</key><array>
<dict><key>Name</key><string>Library</string><key>Master</key><true/><key>Playlist ID</key><integer>1</integer><key>Playlist Items</key><array><dict><key>Track ID</key><integer>101</integer></dict></array></dict>
<dict><key>Name</key><string>Music</string><key>Distinguished Kind</key><integer>4</integer><key>Playlist Persistent ID</key><string>M</string></dict>
<dict><key>Name</key><string>DJ Sets</string><key>Folder</key><true/><key>Playlist Persistent ID</key><string>F1</string></dict>
<dict><key>Name</key><string>Friday</string><key>Playlist Persistent ID</key><string>P1</string><key>Parent Persistent ID</key><string>F1</string><key>Playlist Items</key><array><dict><key>Track ID</key><integer>102</integer></dict><dict><key>Track ID</key><integer>101</integer></dict></array></dict>
</array></dict></plist>"#)).unwrap();
    match import::load(&it, &[]) { Ok(l) => { let t = &l.tracks; let f = l.root.at(&[0, 0]);
        let pass = l.source == Source::ITunes && t.len() == 2 && t[0].bpm == Some(124.0) && t[1].title == "C & D" && t.iter().all(|x| x.exists) && l.root.children.len() == 1 && l.root.children[0].folder && f.is_some_and(|f| f.name == "Friday" && f.tracks == vec![1, 0]);
        chk("iTunes library XML", pass, format!("{} tracks, tree: {} > {}", t.len(), names(&l.root), l.root.children.first().map(names).unwrap_or_default())); }
        Err(e) => chk("iTunes library XML", false, e) }
    // Serato database V2 + nested crate
    let rec = |tag: &str, body: &[u8]| { let mut v = tag.as_bytes().to_vec(); v.extend((body.len() as u32).to_be_bytes()); v.extend(body); v };
    let u16s = |s: &str| s.encode_utf16().flat_map(|c| c.to_be_bytes()).collect::<Vec<u8>>();
    let relm = music.strip_prefix(&dir).unwrap().display().to_string();
    let sd = dir.join("_Serato_"); std::fs::create_dir_all(sd.join("Subcrates")).unwrap();
    let mut db = rec("vrsn", &u16s("2.0/Serato Scratch LIVE Database"));
    for (f, ti, bpm, key) in [("a.mp3", "Alpha", "124.00", "Am"), ("b.flac", "Beta", "70.0", "C")] {
        let mut o = rec("ttyp", &u16s("mp3")); o.extend(rec("pfil", &u16s(&format!("{relm}/{f}")))); o.extend(rec("tsng", &u16s(ti))); o.extend(rec("tbpm", &u16s(bpm))); o.extend(rec("tkey", &u16s(key))); o.extend(rec("tlen", &u16s("05:00.00")));
        db.extend(rec("otrk", &o)); }
    std::fs::write(sd.join("database V2"), &db).unwrap();
    let mut cr = rec("vrsn", &u16s("1.0/Serato ScratchLive Crate"));
    for f in ["b.flac", "sub dir/c & d.wav"] { cr.extend(rec("otrk", &rec("ptrk", &u16s(&format!("{relm}/{f}"))))); }
    std::fs::write(sd.join("Subcrates/House%%Deep.crate"), &cr).unwrap();
    std::fs::write(sd.join("Subcrates/Techno.crate"), &rec("vrsn", &u16s("1.0"))).unwrap();
    let sb = fp(&sd.join("database V2"));
    match import::load(&sd.join("database V2"), &[]) { Ok(l) => { let t = &l.tracks; let deep = l.root.at(&[0, 0]);
        let pass = l.source == Source::Serato && t.len() == 3 && t[0].title == "Alpha" && t[0].bpm == Some(124.0) && t[1].key.as_deref() == Some("C") && (t[0].duration - 300.0).abs() < 1e-9 && t.iter().all(|x| x.exists)
            && l.root.children.len() == 2 && l.root.children[0].name == "House" && l.root.children[0].folder && deep.is_some_and(|d| d.name == "Deep" && d.tracks == vec![1, 2]);
        chk("Serato database V2 + crates", pass, format!("{} tracks (1 only in a crate), crates: {} > {}", t.len(), names(&l.root), l.root.children.first().map(names).unwrap_or_default())); }
        Err(e) => chk("Serato database V2 + crates", false, e) }
    // Engine DJ m.db
    let ed = dir.join("Engine Library/Database2"); std::fs::create_dir_all(&ed).unwrap(); let mdb = ed.join("m.db");
    { let c = rusqlite::Connection::open(&mdb).unwrap();
      c.execute_batch(&format!("CREATE TABLE Track (id INTEGER PRIMARY KEY, path TEXT, filename TEXT, title TEXT, artist TEXT, album TEXT, genre TEXT, bpm INTEGER, bpmAnalyzed REAL, key INTEGER, length REAL);
        CREATE TABLE Playlist (id INTEGER PRIMARY KEY, title TEXT, parentListId INTEGER, isPersisted BOOLEAN, nextListId INTEGER);
        CREATE TABLE PlaylistEntity (id INTEGER PRIMARY KEY, listId INTEGER, trackId INTEGER, databaseUuid TEXT, nextEntityId INTEGER, membershipReference INTEGER);
        INSERT INTO Track VALUES (1, '../../{relm}/a.mp3', 'a.mp3', 'Alpha', 'DJ A', '', 'House', 124, 123.98, 1, 300.0);
        INSERT INTO Track VALUES (2, '../../{relm}/b.flac', 'b.flac', '', 'DJ B', '', '', 70, NULL, 22, 200.0);
        INSERT INTO Playlist VALUES (1, 'Sets', 0, 1, 3); INSERT INTO Playlist VALUES (2, 'Late', 1, 1, 0); INSERT INTO Playlist VALUES (3, 'Chill', 0, 1, 0);
        INSERT INTO PlaylistEntity VALUES (1, 2, 2, 'u', 2, 0); INSERT INTO PlaylistEntity VALUES (2, 2, 1, 'u', 0, 0); INSERT INTO PlaylistEntity VALUES (3, 3, 1, 'u', 0, 0);")).unwrap(); }
    let eb = fp(&mdb);
    match import::load(&mdb, &[]) { Ok(l) => { let t = &l.tracks; let late = l.root.at(&[0, 0]);
        let pass = l.source == Source::Engine && t.len() == 2 && t[0].bpm == Some(123.98) && t[1].bpm == Some(70.0) && t[0].key.as_deref() == Some("8A") && t[1].key.as_deref() == Some("7B") && t[1].title == "b.flac" && t.iter().all(|x| x.exists)
            && l.root.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>() == vec!["Sets", "Chill"] && late.is_some_and(|x| x.name == "Late" && x.tracks == vec![1, 0]);
        chk("Engine DJ m.db", pass, format!("{} tracks, keys {:?}/{:?}, tree: {} > {}", t.len(), t[0].key, t[1].key, names(&l.root), l.root.children.first().map(names).unwrap_or_default())); }
        Err(e) => chk("Engine DJ m.db", false, e) }
    let untouched = fp(&rb) == before && fp(&sd.join("database V2")) == sb && fp(&mdb) == eb && !ed.join("m.db-wal").exists() && !ed.join("m.db-shm").exists() && !ed.join("m.db-journal").exists();
    chk("source files are never modified", untouched, "content and mtime unchanged, no sqlite side files".into());
    // auto-detect usual locations (home + mounted Windows drive)
    let home = dir.join("home"); std::fs::create_dir_all(home.join("Documents")).unwrap(); std::fs::copy(&rb, home.join("Documents/rekordbox.xml")).unwrap();
    let mnt = dir.join("mnt"); let win = mnt.join("win/Users/Mike"); std::fs::create_dir_all(win.join("Documents/Native Instruments/Traktor 3.11.1")).unwrap(); std::fs::copy(&nml, win.join("Documents/Native Instruments/Traktor 3.11.1/collection.nml")).unwrap();
    std::fs::create_dir_all(mnt.join("usb/_Serato_")).unwrap(); std::fs::copy(sd.join("database V2"), mnt.join("usb/_Serato_/database V2")).unwrap();
    let found = import::detect(&home, &[mnt.clone()]);
    let kinds: Vec<_> = found.iter().filter_map(|p| import::sniff(p)).collect();
    chk("auto-detect locations", found.len() == 3 && kinds.contains(&Source::Rekordbox) && kinds.contains(&Source::Traktor) && kinds.contains(&Source::Serato), format!("{} found: {:?}", found.len(), kinds));
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

fn import_tests() -> bool {
    let mut ok = import_tests_core();
    let mut m = model::Mix::default(); let j0 = serde_json::to_string(&m).unwrap();
    m.cue_marks.insert("a.mp3".into(), vec![(64.5, "Drop".into())]); let j = serde_json::to_string(&m).unwrap();
    let back: model::Mix = serde_json::from_str(&j).unwrap(); let old = serde_json::from_str::<model::Mix>(&j0.replace(",\"cue_marks\":{}", ""));
    let pass = back.cue_marks.get("a.mp3").is_some_and(|v| v[0].1 == "Drop") && old.is_ok() && !j0.replace(",\"cue_marks\":{}", "").contains("cue_marks");
    ok &= pass; println!("{} import cue markers saved in the project: old projects load", if pass { "OK  " } else { "FAIL" });
    ok
}

fn ratio_tests() -> bool {
    use crate::model::RATIOS;
    let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} tempo jump {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let sr = 44100u32; let lead = 0.5f32;
    let mk = |bpm: f32, secs: f32| { let (l, r) = crate::testgen::synth_drift(bpm, bpm, 36, true, secs, lead, sr); Arc::new(analysis::Audio { sr, l, r }) };
    let track = |bpm: f64, dur: f64, lane: usize, start: f64| model::Track { path: format!("{bpm}"), title: "k".into(), artist: "k".into(), bpm, offset: lead as f64, key: "8A".into(), duration: dur, start_beat: start, lane, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
    let onsets = |x: &[f32]| { let blk = (sr / 1000) as usize; let bm: Vec<f32> = x.chunks(blk).map(|c| c.iter().fold(0f32, |a, v| a.max(v.abs()))).collect();
        let pk = bm.iter().fold(0f32, |a, v| a.max(*v)); let mut v = vec![]; let mut last = -100i64;
        for j in 25..bm.len() { let prev = bm[j - 25..j].iter().fold(0f32, |a, b| a.max(*b));
            if bm[j] > 0.15 * pk && bm[j] > 1.6 * prev && j as i64 - last > 40 { v.push((j * blk) as f64 / sr as f64); last = j as i64; } } v };
    // onset error of `b` (ratio k) against the mix grid, rendered alone and with a 140 BPM track before it
    let run = |bpm_b: f64, ratio: u8, tempo_mode: u8, with_map: bool| -> (f64, f64, usize, f64) {
        let mut m = model::Mix::default(); m.bpm = 140.0; m.tempo_mode = tempo_mode; m.decks[1].vol = 1.0;
        let a = track(140.0, 20.0, 0, 0.0); let mut b = track(bpm_b, 40.0, 1, 64.0); b.trans.ratio = ratio;
        if with_map { b.map = Some(Arc::new(crate::beatmap::BeatMap { beats: (0..60).map(|k| lead as f64 + k as f64 * 60.0 / bpm_b).collect(), bpm: bpm_b })); }
        m.tracks = vec![a, b]; m.resolve_ratios();
        let mult = m.tracks[1].beat_mult; let bpm_mid = m.bpm_at(80.0);
        let auds = vec![None, Some(mk(bpm_b as f32, 40.0))];
        let (from, to) = (64.0, 96.0);
        let out = engine::render_offline(Arc::new(engine::Snap::build(&m, &auds)), sr, from, to).0;
        let on = onsets(&out);
        // expected: every track beat = 1/mult mix beats -> times along the tempo curve
        let step = mult; let mut err = 0f64; let mut n = 0;
        for &t in on.iter().filter(|&&t| t > 0.05 && t < m.secs(from, to) - 0.3) {
            let mut best = f64::MAX; let mut k = 0.0; while from + k <= to { best = best.min((m.secs(from, from + k) - t).abs()); k += step; }
            err = err.max(best); n += 1; }
        (err * 1000.0, mult, n, bpm_mid)
    };
    let (e, m1, n, _) = run(70.0, 2, 0, false);
    chk("70 into 140 at 1:2", e < 10.0 && m1 == 2.0 && n >= 14, format!("{n} kicks, max error {e:.2} ms (one track beat = {m1} mix beats)"));
    let (e, m1, n, _) = run(70.0, 2, 0, true);
    chk("70 into 140 at 1:2 with beat map", e < 10.0 && m1 == 2.0 && n >= 14, format!("{n} kicks, max error {e:.2} ms"));
    let (e, m1, n, bpm) = run(70.0, 0, 1, false);
    chk("auto ratio + auto tempo keep 140", e < 10.0 && m1 == 2.0 && (bpm - 140.0).abs() < 0.01, format!("auto picked ×{m1}, tempo stays {bpm:.1} BPM, {n} kicks, max error {e:.2} ms"));
    let (e, m1, n, _) = run(280.0, 3, 0, false);
    chk("2:1 (280 into 140)", e < 10.0 && m1 == 0.5 && n >= 6, format!("{n} kicks, max error {e:.2} ms"));
    let (e, m1, n, _) = run(93.333, 4, 0, false);
    chk("2:3 (93.3 into 140)", e < 10.0 && (m1 - 1.5).abs() < 1e-9 && n >= 10, format!("{n} kicks, max error {e:.2} ms"));
    let (_, m1, _, _) = run(100.0, 0, 0, false); let (_, m2, _, _) = run(66.0, 0, 0, false);
    chk("auto keeps 1:1 when doubling is not clearly better", m1 == 1.0 && m2 == 2.0, format!("100 into 140: ×{m1}, 66 into 140: ×{m2}"));
    // timeline length follows the ratio, old projects load with Auto
    let mut m = model::Mix::default(); m.bpm = 140.0; let mut t = track(70.0, 60.0, 0, 0.0); t.trans.ratio = 2; m.tracks = vec![t]; m.resolve_ratios();
    let len = m.tracks[0].end(140.0) - m.tracks[0].vis_start(140.0);
    let j = serde_json::to_string(&m).unwrap().replace(",\"ratio\":2", "").replace(",\"beat_mult\":2.0", "");
    let old: Result<model::Mix, _> = serde_json::from_str(&j);
    chk("clip length and old projects", (len - 140.0).abs() < 0.01 && old.as_ref().is_ok_and(|o| o.tracks[0].trans.ratio == 0 && o.tracks[0].beat_mult == 1.0) && RATIOS.len() == 6,
        format!("60 s at 70 BPM, 1:2 → {len:.1} mix beats; old JSON → Auto"));
    ok
}

fn grid_mode_tests() -> bool {
    use crate::beatmap::BeatMap;
    let mut ok = true;
    let mut chk = |name: &str, pass: bool, info: String| { ok &= pass; println!("{} beatgrid {name}: {info}", if pass { "OK  " } else { "FAIL" }); };
    let sr = 44100u32;
    let err = |b: &[f64], truth: &[f64]| truth[2..truth.len() - 2].iter().map(|t| b.iter().map(|x| (x - t).abs()).fold(f64::MAX, f64::min)).fold(0.0, f64::max) * 1000.0;
    let mono = |l: &[f32]| l.to_vec();
    // --- flex: jump 100 -> 128 and ramp 100 -> 128
    let jump = crate::testgen::jump_times(100.0, 64, 128.0, 96, 0.5); let jsecs = (*jump.last().unwrap() + 1.0) as f32;
    let (jl, jr) = crate::testgen::synth_kicks(&jump, jsecs, sr);
    let (fb, ch) = analysis::flex_beats(&mono(&jl), sr, 114.0);
    let ej = err(&fb, &jump);
    chk("flex follows a 100 → 128 BPM jump", ej < 10.0 && ch.len() == 1 && (ch[0] - jump[64]).abs() < 3.0, format!("max error {ej:.2} ms over {} kicks, change point at {:.1} s (jump at {:.1} s)", jump.len(), ch.first().copied().unwrap_or(-1.0), jump[64]));
    let ramp = crate::testgen::ramp_times(100.0, 128.0, 60.0, 0.5);
    let (rl, _) = crate::testgen::synth_kicks(&ramp, 61.5, sr);
    let (rb, rch) = analysis::flex_beats(&rl, sr, 114.0); let er = err(&rb, &ramp);
    chk("flex follows a 100 → 128 BPM ramp", er < 10.0, format!("max error {er:.2} ms over {} kicks, {} change points", ramp.len(), rch.len()));
    let an = analysis::analyse(&analysis::Audio { sr, l: jl.clone(), r: jr.clone() }, "j".into(), "j".into(), Path::new("j.wav"));
    let (ef, ed) = (err(&an.flex, &jump), err(&an.beats, &jump));
    chk("analysis stores the flex map", ef < 10.0 && an.flex_map().is_some() && analysis::ANALYSIS_VERSION >= 8, format!("flex {ef:.2} ms vs dynamic map {ed:.1} ms on the jump, cache v{}", analysis::ANALYSIS_VERSION));
    // flex playback on a constant 128 BPM mix
    let onsets = |x: &[f32]| { let blk = (sr / 1000) as usize; let bm: Vec<f32> = x.chunks(blk).map(|c| c.iter().fold(0f32, |a, v| a.max(v.abs()))).collect();
        let pk = bm.iter().fold(0f32, |a, v| a.max(*v)); let mut v = vec![]; let mut last = -100i64;
        for j in 25..bm.len() { let prev = bm[j - 25..j].iter().fold(0f32, |a, b| a.max(*b));
            if bm[j] > 0.15 * pk && bm[j] > 1.6 * prev && j as i64 - last > 40 { v.push((j * blk) as f64 / sr as f64); last = j as i64; } } v };
    let track = |bpm: f64, dur: f64| model::Track { path: "a".into(), title: "a".into(), artist: "a".into(), bpm, offset: 0.5, key: "8A".into(), duration: dur, start_beat: 0.0, lane: 0, trans: Default::default(), map: None, locked: false, trim_in: 0.0, trim_out: 0.0, fade_in: 0.0, fade_out: 0.0, stem_view: 0, stem_mutes: vec![], auto: vec![], stretch: 0, beat_mult: 1.0, grid_mode: 1, stabilize: false, flex: Option::None, imported: Option::None, sections: vec![] };
    let grid_err = |m: &model::Mix, out: &[f32], from: f64, to: f64, step: f64| -> (f64, usize) { let on = onsets(out); let mut e = 0f64; let mut n = 0;
        for &t in on.iter().filter(|&&t| t > 0.05 && t < m.secs(from, to) - 0.3) { let mut best = f64::MAX; let mut k = from.ceil(); while k <= to { best = best.min((m.secs(from, k) - t).abs()); k += step; } e = e.max(best); n += 1; } (e * 1000.0, n) };
    { let mut t = track(114.0, jsecs as f64); t.flex = Some(Arc::new(BeatMap { beats: fb.clone(), bpm: 114.0 })); t.grid_mode = 2; t.offset = jump[0]; t.stretch = 2;
      let mut m = model::Mix::default(); m.bpm = 128.0; m.decks[0].vol = 1.0; m.tracks = vec![t];
      let au = vec![Some(Arc::new(analysis::Audio { sr, l: jl.clone(), r: jr.clone() }))];
      let (a, b) = (40.0, 100.0); let out = engine::render_offline(Arc::new(engine::Snap::build(&m, &au)), sr, a, b).0;
      let (e, n) = grid_err(&m, &out, a, b, 1.0);
      chk("flex grid plays the jump on the mix grid", e < 10.0 && n > 50, format!("{n} kicks across the jump at 128 BPM, max error {e:.2} ms")); }
    // --- stabilize: drifting 120 -> 132 BPM track split in three, intro and outro stabilized (Auto tempo)
    let drift = crate::testgen::ramp_times(120.0, 132.0, 60.0, 0.5);
    let (dl, dr) = crate::testgen::synth_kicks(&drift, 61.5, sr); let dau = vec![Some(Arc::new(analysis::Audio { sr, l: dl, r: dr }))];
    let build = |stab: bool, key_lock: bool| { let mut t = track(126.0, 61.5); t.map = Some(Arc::new(BeatMap { beats: drift.clone(), bpm: 126.0 })); t.stretch = if key_lock { 1 } else { 2 };
        let mut m = model::Mix::default(); m.bpm = 126.0; m.tempo_mode = 1; m.decks[0].vol = 1.0; m.tracks = vec![t];
        let r1 = m.split(0, 32.5).unwrap(); let nb = (drift.len() - 1) as f64; let r2 = m.split(r1, nb - 32.5).unwrap();
        if stab { m.tracks[0].stabilize = true; m.tracks[r2].stabilize = true; } (m, r1, r2) };
    let lin_res = |ts: &[f64]| -> f64 { let n = ts.len() as f64; if n < 3.0 { return f64::MAX; } let xs: Vec<f64> = (0..ts.len()).map(|i| i as f64).collect();
        let (sx, sy, sxx, sxy) = xs.iter().zip(ts).fold((0.0, 0.0, 0.0, 0.0), |a, (x, y)| (a.0 + x, a.1 + y, a.2 + x * x, a.3 + x * y));
        let b = (n * sxy - sx * sy) / (n * sxx - sx * sx); let a = (sy - b * sx) / n; xs.iter().zip(ts).map(|(x, y)| (y - a - b * x).abs()).fold(0.0, f64::max) * 1000.0 };
    for key_lock in [false, true] {
        let mut res = vec![]; let mut info = String::new(); let mut pass = true;
        for stab in [true, false] {
            let (m, r1, r2) = build(stab, key_lock);
            let (a, b) = (m.tracks[0].vis_start(126.0), m.tracks[r2].end(126.0));
            let out = engine::render_offline(Arc::new(engine::Snap::build(&m, &[dau[0].clone(), dau[0].clone(), dau[0].clone()])), sr, a, b).0;
            let on = onsets(&out); let (s1, s2) = (m.secs(a, m.tracks[r1].vis_start(126.0)), m.secs(a, m.tracks[r2].vis_start(126.0)));
            let intro: Vec<f64> = on.iter().copied().filter(|&t| t > 0.05 && t < s1 - 0.05).collect(); let outro: Vec<f64> = on.iter().copied().filter(|&t| t > s2 + 0.05 && t < m.secs(a, b) - 0.5).collect();
            let mid: Vec<f64> = on.iter().copied().filter(|&t| t > s1 + 0.05 && t < s2 - 0.05).collect();
            let (ri, ro, rm) = (lin_res(&intro), lin_res(&outro), lin_res(&mid));
            let (ge, gn) = grid_err(&m, &out, a, b, 1.0);
            // boundaries: largest sample step within 10 ms of each boundary
            let jumpmax = |s: f64| { let c = (s * sr as f64) as usize; let w = (0.01 * sr as f64) as usize; (c - w..c + w).map(|i| (out[i + 1] - out[i]).abs()).fold(0f32, f32::max) };
            let (j1, j2) = (jumpmax(s1), jumpmax(s2));
            let cont = (m.tracks[0].sec_at(m.tracks[r1].vis_start(126.0)) - m.tracks[r1].sec_at(m.tracks[r1].vis_start(126.0))).abs();
            if stab { pass &= ri < 2.0 && ro < 2.0 && ge < 2.0 && gn > 100 && j1 < 0.01 && j2 < 0.01 && cont < 1e-9 && intro.len() > 25 && outro.len() > 25; }
            info += &format!("{}: intro/outro off a straight line by {ri:.2}/{ro:.2} ms, middle {rm:.2} ms, kicks on the mix grid within {ge:.2} ms ({gn}), boundary steps {j1:.4}/{j2:.4}; ", if stab { "stabilized" } else { "not stabilized" });
            res.push(ri);
        }
        pass &= res[1] > res[0];
        chk(&format!("stabilized intro/outro ({})", if key_lock { "key lock" } else { "vinyl" }), pass, info);
    }
    // --- imported grid (Rekordbox TEMPO / Traktor AutoGrid)
    let ib = crate::import::grid_beats(&[(0.025, 124.0, 1), (30.0, 125.0, 1)], 60.0).unwrap_or_default();
    let mut t = track(124.0, 60.0); t.imported = Some(Arc::new(BeatMap { beats: ib.clone(), bpm: 124.0 })); t.offset = 0.03; t.set_grid_mode(3);
    let s0 = t.sec_at(t.start_beat); let s8 = t.sec_at(t.start_beat + 8.0);
    let late = ib.iter().position(|&b| b >= 30.0).map(|i| ib[i]).unwrap_or(0.0);
    chk("imported grid", (s0 - 0.025).abs() < 1e-9 && (s8 - (0.025 + 8.0 * 60.0 / 124.0)).abs() < 1e-9 && (late - 30.0).abs() < 1e-9 && ib.len() > 100, format!("{} beats, downbeat {s0:.3} s, tempo change at {late:.3} s", ib.len()));
    // --- modes per clip, manual editing, old projects
    let mut m = model::Mix::default(); m.bpm = 126.0; let mut t = track(126.0, 61.5); t.map = Some(Arc::new(BeatMap { beats: drift.clone(), bpm: 126.0 })); m.tracks = vec![t];
    let r = m.split(0, 40.0).unwrap(); m.tracks[0].set_grid_mode(0);
    let fixed_lin = (m.tracks[0].sec_at(10.0) - (0.5 + 10.0 * 60.0 / 126.0)).abs() < 1e-9; let dyn_right = m.tracks[r].grid().dynamic();
    m.tracks[r].edit_map(|bm| bm.shift(0.001)); let manual = m.tracks[r].grid_mode == 4;
    let j = serde_json::to_string(&m).unwrap(); let back: model::Mix = serde_json::from_str(&j).unwrap();
    let old = j.replace(",\"grid_mode\":0", "").replace(",\"grid_mode\":4", "").replace(",\"stabilize\":false", "");
    let o: Result<model::Mix, _> = serde_json::from_str(&old);
    chk("grid mode per clip", fixed_lin && dyn_right && manual && back.tracks[0].grid_mode == 0 && o.is_ok_and(|o| o.tracks.iter().all(|t| t.grid_mode == 1 && !t.stabilize)) && model::GRID_MODES.len() == 5,
        "split clips Fixed + Dynamic, editing switches to Manual, saved per clip, old projects → Dynamic".into());
    ok
}
