use crate::engine::{Cmd, Engine, Shared};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;

pub struct Audio { pub tx: Sender<Cmd>, pub shared: Arc<Shared>, pub _stream: Option<cpal::Stream>, pub info: String }

pub fn start() -> Audio {
    let (tx, rx) = crossbeam_channel::bounded::<Cmd>(1024);
    let shared = Arc::new(Shared::new());
    match build(rx, shared.clone()) {
        Ok((s, info)) => Audio { tx, shared, _stream: Some(s), info },
        Err(e) => Audio { tx, shared, _stream: None, info: format!("No audio: {e}") },
    }
}

fn build(rx: Receiver<Cmd>, shared: Arc<Shared>) -> anyhow::Result<(cpal::Stream, String)> {
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or_else(|| anyhow::anyhow!("no audio device"))?;
    let def = dev.default_output_config()?;
    let sr = def.sample_rate().0;
    let ch = def.channels() as usize;
    let mut cfg: cpal::StreamConfig = def.config();
    // A fixed 512-frame ALSA buffer implies 128-frame periods (~2.9 ms at 44.1 kHz).
    // Let the backend choose its normal scheduling margin for reliable editor playback.
    cfg.buffer_size = cpal::BufferSize::Default;
    let name = dev.name().unwrap_or_default();
    let mk = |cfg: &cpal::StreamConfig, rx: Receiver<Cmd>, shared: Arc<Shared>| -> anyhow::Result<cpal::Stream> {
        let mut eng = Engine::new(sr as f32, shared);
        let mut tmp: Vec<f32> = vec![0.0; 16384];
        let err = |e| eprintln!("audio error: {e}");
        let s = match def.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(cfg, move |out: &mut [f32], _| {
                while let Ok(c) = rx.try_recv() { eng.handle(c); }
                eng.render(out, ch);
            }, err, None)?,
            cpal::SampleFormat::I16 => dev.build_output_stream(cfg, move |out: &mut [i16], _| {
                while let Ok(c) = rx.try_recv() { eng.handle(c); }
                if tmp.len() < out.len() { tmp.resize(out.len(), 0.0); }
                eng.render(&mut tmp[..out.len()], ch);
                for (o, x) in out.iter_mut().zip(tmp.iter()) { *o = (x * 32767.0) as i16; }
            }, err, None)?,
            f => anyhow::bail!("audio format {f:?} not supported"),
        };
        Ok(s)
    };
    let stream = match mk(&cfg, rx.clone(), shared.clone()) {
        Ok(s) => s,
        Err(_) => { let c2: cpal::StreamConfig = def.config(); mk(&c2, rx, shared)? }
    };
    stream.play()?;
    Ok((stream, format!("{name} · {sr} Hz")))
}

#[cfg(test)]
mod device_diagnostics {
    use super::*;
    #[test]
    #[ignore = "requires local saved mix and active audio device; plays the transition"]
    fn saved_mix_device_clock_keeps_real_time() {
        let path=std::env::var("MIXSTUDIO_DIAGNOSTIC_PROJECT").unwrap();
        let mix:crate::model::Mix=serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let audio:Vec<_>=mix.tracks.iter().map(|t|Some(Arc::new(crate::analysis::load(std::path::Path::new(&t.path)).unwrap().0))).collect();
        let out=start(); println!("Device: {}",out.info); assert!(out._stream.is_some());
        let from=mix.transitions()[0].2+24.0;
        out.tx.send(Cmd::Snap(Arc::new(crate::engine::Snap::build(&mix,&audio)))).unwrap();
        out.tx.send(Cmd::Seek(from)).unwrap();out.tx.send(Cmd::Play(true)).unwrap();
        let begin=std::time::Instant::now();
        for _ in 0..250 { out.tx.send(Cmd::Snap(Arc::new(crate::engine::Snap::build(&mix,&audio)))).unwrap();std::thread::sleep(std::time::Duration::from_millis(16)); }
        let elapsed=begin.elapsed().as_secs_f64(); let progressed=(out.shared.beat()-from)*60.0/mix.bpm;
        out.tx.send(Cmd::Play(false)).unwrap();
        println!("Device wall time {elapsed:.3}s, audio timeline progressed {progressed:.3}s, ratio {:.3}",progressed/elapsed);
        assert!((progressed/elapsed-1.0).abs()<0.05,"audio device playback clock is running too slow or fast");
    }
}
