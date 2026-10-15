# MixStudio

**A DAW-style DJ mixing app written in Rust, where every song is its own clip on a timeline.**

![Status: Alpha 0.5](https://img.shields.io/badge/status-alpha%200.5.1-e8e000?style=flat-square&labelColor=24262b)
![Rust](https://img.shields.io/badge/rust-stable-24262b?style=flat-square)
![License: MIT](https://img.shields.io/badge/license-MIT-24262b?style=flat-square)

> ⚠️ **Alpha 0.5:** MixStudio works and is fun to use, but it's early. Expect rough edges, and keep your mixes backed up.

![MixStudio timeline with a transition](docs/screenshots/transition.png)

## Why

Live DJ software is built for performing: two decks, a crossfader, and one shot to get it right. Planning a mix is different. You want to see the whole set at once, move songs around, try a transition three ways and keep the best one. MixStudio treats a DJ mix like a DAW project. Tracks alternate between Deck 1 and Deck 2 on a timeline, the overlaps become transitions you can edit, and everything stays in sync with one mix tempo.

It's native, fast, and doesn't need an account. No sign-up, no license keys, no cloud. It's your music on your machine.

## Features (Alpha 0.5)

- **Timeline decks:** songs alternate on Deck 1 and Deck 2 as coloured clips with 3-band waveforms (low/mid/high), beat and bar lines, bar numbers and rough section tags (DOWN / CHORUS / UP). There's an overview strip, a time ruler, zoom (Ctrl+scroll), scrolling, a playhead, click-to-seek, and clip dragging with snapping (off / beat / half bar / bar).
- **Automatic analysis:** global BPM (tempogram + full-track comb + beat regression, about 0.02 BPM), musical key (chromagram plus Krumhansl profiles) shown as **Camelot** codes, and bar 1. Results and waveforms are cached and re-analysed automatically when the analysis improves.
- **Dynamic beatgrids:** every beat is tracked (dynamic-programming beat tracking, refined to the kick attack on a ripple-free low-band envelope and smoothed over 16 beats, so soft intro kicks and clicky main kicks land on the same grid), so tracks that drift in tempo (live-played, older or AI-generated material) still stay locked. The Track tab shows the tempo range the beat map follows. Projects saved before 0.3 load with the old fixed grid.
- **Beat-locked time-stretch:** every track plays at the mix tempo through its beat map, so the stretch ratio changes continuously and each beat lands on a mix beat (kicks within a few ms in the self-tests, even against a drifting track). Key lock (on by default) uses a built-in WSOLA stretcher that keeps the pitch; grains containing a kick or snare attack are placed exactly on the beat map, so attacks don't shift when a track is sped up or slowed down. Switch it off for classic varispeed. Playback and export share the same engine.
- **Editable beatgrid:** nudge ±1/±10 ms, Alt+drag the grid on a clip, set the downbeat (Shift+click a beat line or "Set downbeat at playhead"). These work on beat maps too. BPM ×2 / ÷2, **tap tempo** (a hand-set BPM switches the track to a fixed grid), and reset to analysis.
- **AI stem separation:** one click, or automatically for new tracks, splits a song into **drums, bass, melody and vocals** with [HT-Demucs](https://github.com/facebookresearch/demucs). It runs in the background through ONNX Runtime, on your **NVIDIA GPU via CUDA** when available, otherwise on the CPU. Stems are cached on disk, so each track is separated only once.
- **Per-stem transitions:** when both tracks have stems, the overlap shows four real stem lanes, and the Drums / Bass / Melody / Vocals choices (**None / Swap / Crossfade / Full**) drive the audio in playback and export. Swap the bass at the drop, keep the incoming vocals out, or let the drums ride. Without stems, the preset's volume curve is used.
- **Transitions:** overlaps become transition regions with presets that act on the audio: **Crossfade**, **Bass swap**, **High to low**, **Low to high** (filter sweeps) and **Echo out**. You can set the overlap length (4/8/16/32 bars), and the volume curves are drawn on the clips.
- **Mixer:** 3-band EQ (High/Mid/Low), filter (LP ↔ HP), volume, solo and mute per deck, plus level meters and a master meter.
- **Auto-order:** one click reorders the playlist for smooth transitions. Every pair of tracks is scored on Camelot key (same key, ±1, relative A/B, with energy-boost moves +2 / +7 weighted lower) and tempo (including half/double time). An optional energy curve (build up, peak in the middle, or flat) is also scored. A greedy start plus simulated annealing and 2-opt finds the order, taking about 20 ms for 100 tracks. Choose *Balanced*, *Harmonic first* or *Tempo first* in the ⏷ menu next to the button. Keep the first track in place, or lock any track with 🔒 in the playlist. The layout and beat-matched transitions are rebuilt, and Undo restores the old order.
- **Compatibility indicators:** every transition shows a green / yellow / red badge with the key move and tempo change (e.g. `8A→9A ±1 · +1.6%`), on the timeline and in the playlist's *Next transition* column.
- **Library:** add your music folders once (📁 *Add folder*). They're scanned recursively in the background (mp3, wav, flac, ogg, m4a/aac, aiff), and title, artist, album, genre and duration are read from the tags. The folder list is saved in `~/.config/mixstudio/library.json` and rescanned at every start; the index is incremental (path, size and modification time), so only new or changed files are read and removed files disappear. BPM and key are analysed in the background with limited concurrency (shared with the normal analysis cache) and a progress bar. The table is sortable (Title, Artist, BPM, Key, Duration, Genre, Folder), has a search box and **key-compatible** / **BPM ±%** filters relative to the selected (or current) track, and stays fast with 10 000+ files (virtualized rows). Double-click, *Add to mix*, or drag the selection onto the timeline; Ctrl/Shift for multi-select. Unplugged or failing drives show as *not available* / *not responding*; every disk access has a timeout, so a dying disk can't hang the scan, and their tracks stay in the index until the drive is back.
- **Sample lanes:** drop extra audio on Sample 1/2. It's synced to the mix tempo.
- **Metronome:** a click on mix beats, accented on the downbeat, with its own volume. It's never exported.
- **Undo/redo**, **save/load mixes as JSON**, **WAV export** of the whole mix, and **drag and drop** of audio files (mp3, wav, flac, ogg, m4a).

| Transition tab | Track / Beatgrid tab | Playlist |
|---|---|---|
| ![Transition tab](docs/screenshots/transition.png) | ![Track tab](docs/screenshots/track.png) | ![Playlist](docs/screenshots/playlist.png) |

![Whole mix overview](docs/screenshots/wide.png)

![Per-stem transition](docs/screenshots/stems.png)

![Auto-order and compatibility badges](docs/screenshots/autoorder.png)

## Roadmap

- Per-stem automation curves and stem solo/mute on clips
- Higher-quality stem models (HT-Demucs fine-tuned bag, MDX-Net)
- Rubber Band-quality time-stretching
- Automix that picks transition points (phrases, energy) and transition presets
- Auto-order across a whole library (build a set from a crate), with BPM-range and must-include constraints
- Tempo automation in the tempo lane
- Editing single beats in the beat map (warp markers) and non-4/4 meters
- MIDI controller support
- Headphone cue / pre-listen
- Library: watch folders for changes live (filesystem watcher); for now, rescan at start or with *Rescan now*
- Video export
- Rekordbox / Serato export
- VST effects

## Platforms

MixStudio is built with [egui/eframe](https://github.com/emilk/egui) and [cpal](https://github.com/RustAudio/cpal), so it's cross-platform by design: **Windows, Linux and macOS**. The Alpha is tested on Linux (CachyOS, KDE Plasma). Windows and macOS builds are coming.

On Linux, MixStudio prefers X11/XWayland when it's available, because winit doesn't support file drag and drop on native Wayland yet. Set `MIXSTUDIO_WAYLAND=1` to force native Wayland.

## Building

You need stable Rust (1.92 or newer).

```sh
# Linux: ALSA development headers
#   Arch/CachyOS: sudo pacman -S alsa-lib
#   Debian/Ubuntu: sudo apt install libasound2-dev
git clone https://github.com/soulspin/MixStudio
cd MixStudio
cargo build --release
./target/release/mixstudio
```

### Stem separation (GPU or CPU)

Stem separation uses ONNX Runtime, which MixStudio loads at runtime, so the app builds and runs without it. Install it to enable stems:

```sh
# Arch / CachyOS with an NVIDIA GPU (pulls in cuda + cudnn):
sudo pacman -S onnxruntime-cuda
# CPU only:
sudo pacman -S onnxruntime-cpu
# Anywhere else: download onnxruntime from github.com/microsoft/onnxruntime/releases
# and point MixStudio at it:
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
```

The first time you separate a track, MixStudio downloads the model (166 MB, verified by SHA-256) to `~/.local/share/mixstudio/models`. Stems are cached in `~/.cache/mixstudio/stems` (about 30 MB per minute of audio). Set `MIXSTUDIO_CPU=1` to force CPU inference.

**Model and license:** `htdemucs_fp16weights.onnx` from [StemSplitio/htdemucs-onnx](https://huggingface.co/StemSplitio/htdemucs-onnx) (pinned revision). It's an ONNX export of HT-Demucs by Meta AI Research ([facebookresearch/demucs](https://github.com/facebookresearch/demucs)). Both the original weights/code and the export are **MIT-licensed**. The model isn't bundled with MixStudio.

Want to try it without your own music? `mixstudio --demo /tmp/ms-demo` generates four test tracks and builds a demo mix. `mixstudio --selftest /tmp/ms-test` runs the analysis, stretch and render self-tests.

### Command line

`mixstudio --analyse <files>` prints BPM, downbeat and key. `mixstudio --align <a> <b>` mixes two tracks and measures kick alignment in the overlap. `mixstudio --selftest <dir>` runs the test suite.

## Usage

1. Open the **Library** tab, add your music folders, and double-click or drag tracks onto the timeline. Or click **+** (Add tracks) or drop audio files on the window. Each new song goes on the next deck and overlaps the previous one by 16 bars.
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

- Stem separation on the CPU is slower than real time on long tracks (about 1–2× real time on 8 cores). A GPU is much faster.
- Stems are held in memory as 16-bit audio (about 30 MB per minute per track).
- WSOLA is good, but not Rubber Band quality. Big tempo changes on pads and vocals can sound grainy.
- Beat maps follow gradual drift, not sudden tempo jumps or rubato.
- Analysis is tuned for 4/4 dance music. Some tracks are detected at half or double tempo (use ×2 / ÷2 or tap tempo).
- Section tags are a rough energy guess.
- File dialogs use the XDG desktop portal (KDE/GNOME), then kdialog, then zenity. If none works, the status line says why: type or paste a folder path in the Library tab, or drop a folder on the window. `MIXSTUDIO_DIALOG=kdialog` (or portal/zenity) forces one; `mixstudio --dialog-check` lists what is available.
- Carousel, Video, effects beyond EQ/filter/echo, and headphone cue aren't implemented yet.
- Decoded audio is held in memory (about 100 MB per 6-minute track).

## Not affiliated

MixStudio is inspired by the timeline approach of DJ.Studio, but it's an independent project written from scratch. It isn't affiliated with, endorsed by, or connected to DJ.Studio, and it contains no code or assets from it.

## License

[MIT](LICENSE) © 2026 soulspin
