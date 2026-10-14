# MixStudio

**A DAW-style DJ mixing app written in Rust, where every song is its own clip on a timeline.**

![Status: Alpha 0.1](https://img.shields.io/badge/status-alpha%200.1-e8e000?style=flat-square&labelColor=24262b)
![Rust](https://img.shields.io/badge/rust-stable-24262b?style=flat-square)
![License: MIT](https://img.shields.io/badge/license-MIT-24262b?style=flat-square)

> ⚠️ **Alpha 0.1:** MixStudio works and is fun to use, but it's early. Expect rough edges, and keep your mixes backed up.

![MixStudio timeline with a transition](docs/screenshots/transition.png)

## Why

Live DJ software is built for performing: two decks, a crossfader, and one shot to get it right. Planning a mix is different. You want to see the whole set at once, move songs around, try a transition three ways and keep the best one. MixStudio treats a DJ mix like a DAW project. Tracks alternate between Deck 1 and Deck 2 on a timeline, the overlaps become transitions you can edit, and everything stays in sync with one mix tempo.

It's native, fast, and doesn't need an account. No sign-up, no license keys, no cloud. It's your music on your machine.

## Features (Alpha 0.1)

- **Timeline decks:** songs alternate on Deck 1 and Deck 2 as coloured clips with 3-band waveforms (low/mid/high), beat and bar lines, bar numbers and rough section tags (DOWN / CHORUS / UP). There's an overview strip, a time ruler, zoom (Ctrl+scroll), scrolling, a playhead, click-to-seek, and clip dragging with snapping (off / beat / half bar / bar).
- **Automatic analysis:** BPM (onset envelope plus autocorrelation and a comb search), first downbeat / bar 1 (kick and bass energy per beat phase, attack strength and phrase changes), and musical key (chromagram plus Krumhansl profiles) shown as **Camelot** codes. Results and waveforms are cached.
- **Key-lock time-stretch:** every track plays at the mix tempo. Key lock (on by default) uses a built-in WSOLA stretcher that keeps the pitch. Switch it off for classic varispeed. Playback and export share the same engine.
- **Editable beatgrid:** nudge ±1/±10 ms, Alt+drag the grid on a clip, set the downbeat (Shift+click a beat line or "Set downbeat at playhead"), BPM ×2 / ÷2, **tap tempo**, and reset to analysis.
- **Transitions:** overlaps become transition regions with presets that act on the audio: **Crossfade**, **Bass swap**, **High to low**, **Low to high** (filter sweeps) and **Echo out**. You can set the overlap length (4/8/16/32 bars), and the volume curves are drawn on the clips.
- **Mixer:** 3-band EQ (High/Mid/Low), filter (LP ↔ HP), volume, solo and mute per deck, plus level meters and a master meter.
- **Harmonize:** reorders the playlist by Camelot compatibility and tempo, then lays the mix out again.
- **Sample lanes:** drop extra audio on Sample 1/2. It's synced to the mix tempo.
- **Metronome:** a click on mix beats, accented on the downbeat, with its own volume. It's never exported.
- **Undo/redo**, **save/load mixes as JSON**, **WAV export** of the whole mix, and **drag and drop** of audio files (mp3, wav, flac, ogg, m4a).

| Transition tab | Track / Beatgrid tab | Playlist |
|---|---|---|
| ![Transition tab](docs/screenshots/transition.png) | ![Track tab](docs/screenshots/track.png) | ![Playlist](docs/screenshots/playlist.png) |

![Whole mix overview](docs/screenshots/wide.png)

## Roadmap

- Real AI stem separation (Demucs / MDX-Net via ONNX on the GPU)
- Per-stem transitions (Drums / Bass / Melody / Vocals: None, Swap, Crossfade, Full). The UI is already there.
- Rubber Band-quality time-stretching
- Automix that picks transition points (phrases, energy)
- Tempo automation in the tempo lane
- MIDI controller support
- Headphone cue / pre-listen
- A library browser
- Video export
- Rekordbox / Serato export
- VST effects

## Platforms

MixStudio is built with [egui/eframe](https://github.com/emilk/egui) and [cpal](https://github.com/RustAudio/cpal), so it's cross-platform by design: **Windows, Linux and macOS**. The Alpha is tested on Linux (CachyOS, KDE Plasma). Windows and macOS builds are coming.

On Linux, MixStudio prefers X11/XWayland when it's available, because winit doesn't support file drag and drop on native Wayland yet. Set `MIXSTUDIO_WAYLAND=1` to force native Wayland.

## Building

You need stable Rust (1.85 or newer).

```sh
# Linux: ALSA development headers
#   Arch/CachyOS: sudo pacman -S alsa-lib
#   Debian/Ubuntu: sudo apt install libasound2-dev
git clone https://github.com/soulspin/MixStudio
cd MixStudio
cargo build --release
./target/release/mixstudio
```

Want to try it without your own music? `mixstudio --demo /tmp/ms-demo` generates four test tracks and builds a demo mix. `mixstudio --selftest /tmp/ms-test` runs the analysis, stretch and render self-tests.

## Usage

1. Click **+** (Add tracks) or drop audio files on the window. Each new song goes on the next deck and overlaps the previous one by 16 bars.
2. Drag clips to adjust overlaps, and click a blue transition bar to choose a preset in the **Transition** tab.
3. Fix any beatgrid in the **Track** tab, and set the mix tempo and key lock in **Master**.
4. Press **Harmonize** for a key-friendly order, then **Export** to render a WAV.

| Shortcut | Action |
|---|---|
| Space | Play / pause |
| Ctrl + scroll | Zoom around the pointer |
| Scroll | Move in time |
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo |
| Delete | Remove the selected track |
| Alt + drag on a clip | Move the beatgrid |
| Shift + click on a beat line | Set the downbeat |
| Double-click a knob / fader | Reset |

## Known limitations

- Stems are visual only. Transitions act on the full track.
- WSOLA is good, but not Rubber Band quality. Big tempo changes on pads and vocals can sound grainy.
- Analysis is tuned for 4/4 dance music. Some tracks are detected at half or double tempo (use ×2 / ÷2 or tap tempo).
- Section tags are a rough energy guess.
- Carousel, Library, Video, effects beyond EQ/filter/echo, and headphone cue aren't implemented yet.
- Decoded audio is held in memory (about 100 MB per 6-minute track).

## Not affiliated

MixStudio is inspired by the timeline approach of DJ.Studio, but it's an independent project written from scratch. It isn't affiliated with, endorsed by, or connected to DJ.Studio, and it contains no code or assets from it.

## License

[MIT](LICENSE) © 2026 soulspin
