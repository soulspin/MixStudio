//! Mix export: WAV 16/24-bit, FLAC (pure-Rust flacenc) and MP3 (through ffmpeg when installed).
use std::path::Path;

pub const FORMATS: [(&str, &str); 4] = [("WAV 16-bit", "wav"), ("WAV 24-bit", "wav"), ("FLAC 24-bit", "flac"), ("MP3 320 kbps (ffmpeg)", "mp3")];

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
    let tmp = path.with_extension("mixstudio-tmp.wav");
    wav(&tmp, l, r, sr, 24).map_err(|e| e.to_string())?;
    let o = std::process::Command::new("ffmpeg").args(["-y", "-loglevel", "error", "-i"]).arg(&tmp).args(["-codec:a", "libmp3lame", "-b:a", "320k"]).arg(path).output();
    let _ = std::fs::remove_file(&tmp);
    match o { Ok(o) if o.status.success() => Ok(()), Ok(o) => Err(format!("ffmpeg failed: {}", String::from_utf8_lossy(&o.stderr).trim())), Err(e) => Err(e.to_string()) }
}

pub fn write(path: &Path, l: &[f32], r: &[f32], sr: u32, fmt: u8) -> Result<(), String> {
    match fmt { 0 => wav(path, l, r, sr, 16).map_err(|e| e.to_string()), 1 => wav(path, l, r, sr, 24).map_err(|e| e.to_string()), 2 => flac(path, l, r, sr), _ => mp3(path, l, r, sr) }
}
