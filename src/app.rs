use crate::audio::decoder::{decode_file, DecodedAudio};
use crate::audio::engine::AudioEngine;
use crate::session::clip::{save_clip, Clip};
use crate::tui::events::{spawn_event_thread, AppEvent};
use crate::tui::ui::render;
use anyhow::Result;
use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct AppState {
    pub pitch: f32,
    pub tempo: f32,
    pub playing: bool,
    pub position_secs: f32,
    pub duration_secs: f32,
    pub in_point: Option<f32>,
    pub out_point: Option<f32>,
    pub looping: bool,
    pub show_help: bool,
    pub files: Vec<PathBuf>,
    pub file_idx: usize,
    pub clips: Vec<Clip>,
    pub clip_counter: u32,
    pub status_msg: Option<(String, Instant)>,
    pub waveform_rms: Vec<u64>,
    pub sample_rate_mismatch: bool,
}

pub struct App {
    state: AppState,
    engine: AudioEngine,
    decoded: Arc<Vec<f32>>,
    sample_rate: u32,
}

impl App {
    pub fn new(files: Vec<PathBuf>, decoded: DecodedAudio) -> Result<Self> {
        let decoded_arc = Arc::new(decoded.samples);
        let sample_rate = decoded.sample_rate;
        let duration_secs = decoded.duration_secs;
        let waveform_rms = compute_rms(&decoded_arc, 200);

        let engine = AudioEngine::new(Arc::clone(&decoded_arc), sample_rate)?;

        let state = AppState {
            pitch: 0.0,
            tempo: 1.0,
            playing: true,
            position_secs: 0.0,
            duration_secs,
            in_point: None,
            out_point: None,
            looping: false,
            show_help: false,
            files,
            file_idx: 0,
            clips: Vec::new(),
            clip_counter: 0,
            status_msg: None,
            waveform_rms,
            sample_rate_mismatch: false,
        };

        Ok(Self {
            state,
            engine,
            decoded: decoded_arc,
            sample_rate,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let (tx, rx) = std::sync::mpsc::channel();
        spawn_event_thread(tx);

        loop {
            // Sync state from engine atomics
            let params = &self.engine.params;
            let pos_samples = params.position_samples.load(Ordering::Relaxed);
            self.state.position_secs = pos_samples as f32 / (self.sample_rate as f32 * 2.0);
            self.state.playing = params.playing.load(Ordering::Relaxed);

            terminal.draw(|f| render(f, &self.state))?;

            match rx.recv_timeout(Duration::from_millis(16)) {
                Ok(AppEvent::Key(key)) => {
                    if self.handle_key(key)? {
                        break;
                    }
                }
                Ok(AppEvent::Tick) => {}
                Err(_) => {}
            }
        }

        // Shutdown
        self.engine.params.stop_flag.store(true, Ordering::Relaxed);
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        Ok(())
    }

    /// Returns true if the app should quit.
    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        let params = &self.engine.params;
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            KeyCode::Char('q') => return Ok(true),

            KeyCode::Char('?') => {
                self.state.show_help = !self.state.show_help;
            }

            KeyCode::Char(' ') => {
                let playing = !self.state.playing;
                params.playing.store(playing, Ordering::Relaxed);
                self.state.playing = playing;
            }

            KeyCode::Up => {
                let step = if shift { 2.0 } else { 0.5 };
                self.state.pitch = (self.state.pitch + step).clamp(-24.0, 24.0);
                params
                    .pitch_semitones
                    .store(self.state.pitch, Ordering::Relaxed);
            }
            KeyCode::Down => {
                let step = if shift { 2.0 } else { 0.5 };
                self.state.pitch = (self.state.pitch - step).clamp(-24.0, 24.0);
                params
                    .pitch_semitones
                    .store(self.state.pitch, Ordering::Relaxed);
            }
            KeyCode::Right => {
                self.state.tempo = (self.state.tempo + 0.05).clamp(0.1, 4.0);
                params
                    .tempo_multiplier
                    .store(self.state.tempo, Ordering::Relaxed);
            }
            KeyCode::Left => {
                self.state.tempo = (self.state.tempo - 0.05).clamp(0.1, 4.0);
                params
                    .tempo_multiplier
                    .store(self.state.tempo, Ordering::Relaxed);
            }

            KeyCode::Char('r') => {
                self.state.pitch = 0.0;
                self.state.tempo = 1.0;
                params.pitch_semitones.store(0.0, Ordering::Relaxed);
                params.tempo_multiplier.store(1.0, Ordering::Relaxed);
            }

            KeyCode::Char('i') => {
                self.state.in_point = Some(self.state.position_secs);
                self.sync_loop_params();
            }
            KeyCode::Char('o') => {
                self.state.out_point = Some(self.state.position_secs);
                self.sync_loop_params();
            }

            KeyCode::Char('[') => {
                if let Some(t) = self.state.in_point {
                    self.seek(t);
                }
            }
            KeyCode::Char(']') => {
                if let Some(t) = self.state.out_point {
                    self.seek(t);
                }
            }

            KeyCode::Char('l') => {
                self.state.looping = !self.state.looping;
                self.sync_loop_params();
            }

            KeyCode::Char('s') => {
                self.save_clip()?;
            }

            KeyCode::Char('n') => {
                self.next_file(1)?;
            }
            KeyCode::Char('p') => {
                self.next_file(-1)?;
            }

            // Presets
            KeyCode::Char('1') => self.apply_preset(-3.0, 0.80),
            KeyCode::Char('2') => self.apply_preset(-6.0, 0.70),
            KeyCode::Char('3') => self.apply_preset(-9.0, 0.60),

            _ => {}
        }
        Ok(false)
    }

    fn seek(&self, secs: f32) {
        let sample = (secs * self.sample_rate as f32 * 2.0) as u64;
        self.engine.params.seek_to.store(sample, Ordering::Relaxed);
    }

    fn sync_loop_params(&self) {
        let params = &self.engine.params;
        params.looping.store(self.state.looping, Ordering::Relaxed);
        params
            .loop_in
            .store(self.state.in_point.unwrap_or(-1.0), Ordering::Relaxed);
        params
            .loop_out
            .store(self.state.out_point.unwrap_or(-1.0), Ordering::Relaxed);
    }

    fn apply_preset(&mut self, pitch: f32, tempo: f32) {
        self.state.pitch = pitch;
        self.state.tempo = tempo;
        let params = &self.engine.params;
        params.pitch_semitones.store(pitch, Ordering::Relaxed);
        params.tempo_multiplier.store(tempo, Ordering::Relaxed);
    }

    fn save_clip(&mut self) -> Result<()> {
        let in_pt = self.state.in_point.unwrap_or(0.0);
        let out_pt = self.state.out_point.unwrap_or(self.state.duration_secs);

        let source = self.state.files[self.state.file_idx].clone();
        self.state.clip_counter += 1;
        let filename = format!("clip_{:03}.wav", self.state.clip_counter);
        let out_path = std::path::Path::new(&filename).to_path_buf();

        let clip = Clip {
            source,
            in_point: in_pt,
            out_point: out_pt,
            pitch: self.state.pitch,
            tempo: self.state.tempo,
        };

        save_clip(&clip, &self.decoded, self.sample_rate, &out_path)?;

        let msg = format!("saved → {filename}");
        tracing::info!("{msg}");
        self.state.status_msg = Some((msg, Instant::now()));
        self.state.clips.push(clip);
        Ok(())
    }

    fn next_file(&mut self, delta: i32) -> Result<()> {
        let n = self.state.files.len();
        if n <= 1 {
            return Ok(());
        }
        let new_idx = ((self.state.file_idx as i32 + delta).rem_euclid(n as i32)) as usize;
        self.state.file_idx = new_idx;

        let path = self.state.files[new_idx].clone();
        let audio = decode_file(&path)?;

        self.sample_rate = audio.sample_rate;
        self.state.duration_secs = audio.duration_secs;
        self.state.waveform_rms = compute_rms(&audio.samples, 200);
        self.state.in_point = None;
        self.state.out_point = None;
        self.state.position_secs = 0.0;

        // Stop old engine
        self.engine.params.stop_flag.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(50));

        let decoded_arc = Arc::new(audio.samples);
        self.decoded = Arc::clone(&decoded_arc);
        self.engine = AudioEngine::new(decoded_arc, self.sample_rate)?;

        // Re-apply current params
        let params = &self.engine.params;
        params
            .pitch_semitones
            .store(self.state.pitch, Ordering::Relaxed);
        params
            .tempo_multiplier
            .store(self.state.tempo, Ordering::Relaxed);

        Ok(())
    }
}

fn compute_rms(samples: &[f32], buckets: usize) -> Vec<u64> {
    if samples.is_empty() || buckets == 0 {
        return vec![0; buckets];
    }
    let chunk_size = (samples.len() / buckets).max(1);
    samples
        .chunks(chunk_size)
        .take(buckets)
        .map(|c| {
            let rms = (c.iter().map(|&s| s * s).sum::<f32>() / c.len() as f32).sqrt();
            (rms * 1000.0) as u64
        })
        .collect()
}
