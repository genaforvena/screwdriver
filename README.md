# screwdriver

A real-time terminal audio tool for chopped & screwed style experimentation.
Written in Rust. No GUI, no mouse — just keypresses and sound.

## What it does

- Load WAV, MP3, FLAC, or OGG files
- Play them back with live pitch and tempo control (via librubberband)
- Hear changes within ~0.5 seconds, no re-render
- Mark chop points while listening and save processed clips to WAV
- Build up a session of clips from multiple source files

## System requirements

Ubuntu 22.04+ (or WSL2) with PipeWire or PulseAudio.

```bash
sudo apt install libasound2-dev libclang-dev librubberband-dev pkg-config
```

Verify rubberband is found:
```bash
pkg-config --libs rubberband   # should print: -lrubberband
```

## Build

```bash
git clone <repo>
cd screwdriver
cargo build --release
```

The binary will be at `target/release/screwdriver`.

## Usage

```bash
screwdriver voice.wav
screwdriver *.wav          # cycle through multiple files with n/p
```

Logs go to `~/.local/share/screwdriver/screwdriver.log`.  
Saved clips go to the current directory as `clip_001.wav`, `clip_002.wav`, etc.

## Key bindings

| Key            | Action                              |
|----------------|-------------------------------------|
| `↑` / `↓`      | Pitch +/- 0.5 semitones             |
| `Shift+↑/↓`    | Pitch +/- 2.0 semitones (coarse)    |
| `←` / `→`      | Tempo +/- 0.05×                     |
| `Space`        | Play / pause                        |
| `i`            | Set in point at current position    |
| `o`            | Set out point at current position   |
| `[`            | Jump to in point                    |
| `]`            | Jump to out point                   |
| `l`            | Toggle loop between in/out points   |
| `s`            | Save current clip as WAV            |
| `n` / `p`      | Next / previous file                |
| `1`            | Preset: -3 st, 0.80× (light screw) |
| `2`            | Preset: -6 st, 0.70× (classic)     |
| `3`            | Preset: -9 st, 0.60× (deep screw)  |
| `r`            | Reset pitch and tempo               |
| `?`            | Toggle help overlay                 |
| `q`            | Quit                                |

## Architecture

```
[decoded audio: Vec<f32>]
        │
        ▼
[processing thread]
  - reads chunks from decoded buffer
  - calls rubberband pitch/tempo on each chunk
  - writes output to ring buffer
        │
        ▼
[ring buffer: ~0.5s of processed audio]
        │
        ▼
[cpal callback] → speakers
```

Parameter changes (↑↓←→) go from TUI → atomics → processing thread, so
there is a small lag (~0.5s) between turning a knob and hearing the change.
