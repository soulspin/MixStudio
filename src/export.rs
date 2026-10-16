//! Mix export: WAV 16/24-bit, FLAC (pure-Rust flacenc) and MP3 (through ffmpeg when installed).
use std::path::Path;

pub const FORMATS: [(&str, &str); 4] = [("WAV 16-bit", "wav"), ("WAV 24-bit", "wav"), ("FLAC 24-bit", "flac"), ("MP3 320 kbps (ffmpeg)", "mp3")];

pub fn end_beat(mix: &crate::model::Mix) -> f64 {
    mix.fx.iter().map(|f|f.end()+0.5).fold(mix.length_beats()+16.0,f64::max)
}

/// Own only a fresh private directory; cleanup cannot unlink adjacent user files.
struct TemporaryWav { dir: std::path::PathBuf, path: std::path::PathBuf }
impl TemporaryWav {
    fn new() -> std::io::Result<Self> {
        let dir=std::env::temp_dir().join(format!("mixstudio-export-{}",crate::recovery::unique_id()));
        let mut builder=std::fs::DirBuilder::new();
        #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
        builder.create(&dir)?;
        Ok(Self {path:dir.join("audio.wav"),dir})
    }
}
impl Drop for TemporaryWav {
    fn drop(&mut self) { let _=std::fs::remove_file(&self.path); let _=std::fs::remove_dir(&self.dir); }
}

pub fn wav(path: &Path, l: &[f32], r: &[f32], sr: u32, bits: u16) -> std::io::Result<()> {
    let n = l.len() as u32; let bps = (bits / 8) as u32; let mut v = Vec::with_capacity(44 + (n * 2 * bps) as usize);
    v.extend(b"RIFF"); v.extend((36 + n * 2 * bps).to_le_bytes()); v.extend(b"WAVEfmt "); v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); v.extend(2u16.to_le_bytes()); v.extend(sr.to_le_bytes()); v.extend((sr * 2 * bps).to_le_bytes());
    v.extend(((2 * bps) as u16).to_le_bytes()); v.extend(bits.to_le_bytes()); v.extend(b"data"); v.extend((n * 2 * bps).to_le_bytes());
    for (a, b) in l.iter().zip(r) { for x in [a, b] { let x = x.clamp(-1.0, 1.0);
        if bits == 24 { let s = (x * 8_388_607.0).round() as i32; v.extend(&s.to_le_bytes()[..3]); } else { v.extend(((x * 32767.0).round() as i16).to_le_bytes()); } } }
    std::fs::write(path, v)
}

pub fn flac(path: &Path, l: &[f32], r: &[f32], sr: u32) -> Result<(), String> {
    use flacenc::component::BitRepr; use flacenc::error::Verify;
    let mut s = Vec::with_capacity(l.len() * 2);
    for (a, b) in l.iter().zip(r) { s.push((a.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32); s.push((b.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32); }
    let cfg = flacenc::config::Encoder::default().into_verified().map_err(|e| format!("{e:?}"))?;
    let src = flacenc::source::MemSource::from_samples(&s, 2, 24, sr as usize);
    let st = flacenc::encode_with_fixed_block_size(&cfg, src, cfg.block_size).map_err(|e| format!("FLAC encode failed: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new(); st.write(&mut sink).map_err(|e| format!("{e:?}"))?;
    std::fs::write(path, sink.as_slice()).map_err(|e| e.to_string())
}

pub fn has_ffmpeg() -> bool { std::process::Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false) }

pub fn mp3(path: &Path, l: &[f32], r: &[f32], sr: u32) -> Result<(), String> {
    if !has_ffmpeg() { return Err("MP3 export needs ffmpeg (Arch/CachyOS: sudo pacman -S ffmpeg). Use WAV or FLAC instead.".into()); }
    let tmp = TemporaryWav::new().map_err(|e| e.to_string())?;
    wav(&tmp.path, l, r, sr, 24).map_err(|e| e.to_string())?;
    let o = std::process::Command::new("ffmpeg").args(["-y", "-loglevel", "error", "-i"]).arg(&tmp.path).args(["-codec:a", "libmp3lame", "-b:a", "320k"]).arg(path).output();
    match o { Ok(o) if o.status.success() => Ok(()), Ok(o) => Err(format!("ffmpeg failed: {}", String::from_utf8_lossy(&o.stderr).trim())), Err(e) => Err(e.to_string()) }
}

pub fn write(path: &Path, l: &[f32], r: &[f32], sr: u32, fmt: u8) -> Result<(), String> {
    match fmt { 0 => wav(path, l, r, sr, 16).map_err(|e| e.to_string()), 1 => wav(path, l, r, sr, 24).map_err(|e| e.to_string()), 2 => flac(path, l, r, sr), _ => mp3(path, l, r, sr) }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    #[test]
    fn export_includes_the_complete_effect_tail_and_release() {
        let mut mix=crate::model::Mix::default();
        let mut echo=crate::fx::FxBlock::new(crate::fx::FxKind::Echo,4,10.0); echo.len=8.0; echo.tail=64.0;
        mix.fx.push(echo);
        assert_eq!(end_beat(&mix),82.5);
    }
    #[test]
    fn mp3_export_does_not_touch_a_same_named_user_wav() {
        if !has_ffmpeg() { return; }
        let dir=std::env::temp_dir().join(format!("mixstudio-mp3-test-{}",crate::recovery::unique_id()));
        std::fs::create_dir(&dir).unwrap();
        let output=dir.join("set.mp3"); let existing=dir.join("set.mixstudio-tmp.wav");
        std::fs::write(&existing,b"user audio - must survive").unwrap();
        mp3(&output,&vec![0.1;4410],&vec![0.1;4410],44100).unwrap();
        assert_eq!(std::fs::read(existing).unwrap(),b"user audio - must survive");
        assert!(std::fs::metadata(output).unwrap().len()>100);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),2,"temporary export files leaked");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
