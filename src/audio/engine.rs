use crate::audio::rubberband::{RubberBand, OPTION_PROCESS_REAL_TIME};
use anyhow::{Context, Result};
use atomic_float::AtomicF32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    traits::{Consumer, Producer, Split},
    HeapRb,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const CHUNK: usize = 1024;

pub struct AudioParams {
    pub pitch_semitones: AtomicF32,
    pub tempo_multiplier: AtomicF32,
    /// Position in interleaved stereo samples.
    pub position_samples: AtomicU64,
    /// Seek target: if != u64::MAX, processing thread seeks to this sample index.
    pub seek_to: AtomicU64,
    pub playing: AtomicBool,
    pub stop_flag: AtomicBool,
    pub looping: AtomicBool,
    pub loop_in: AtomicF32,  // seconds; negative = unset
    pub loop_out: AtomicF32, // seconds; negative = unset
    pub sample_rate: u32,
}

impl AudioParams {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            pitch_semitones: AtomicF32::new(0.0),
            tempo_multiplier: AtomicF32::new(1.0),
            position_samples: AtomicU64::new(0),
            seek_to: AtomicU64::new(u64::MAX),
            playing: AtomicBool::new(true),
            stop_flag: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            loop_in: AtomicF32::new(-1.0),
            loop_out: AtomicF32::new(-1.0),
            sample_rate,
        }
    }
}

pub struct AudioEngine {
    pub params: Arc<AudioParams>,
    pub device_sample_rate: u32,
    _stream: cpal::Stream,
}

impl AudioEngine {
    pub fn new(decoded: Arc<Vec<f32>>, sample_rate: u32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No output device available")?;

        let mut supported_configs = device
            .supported_output_configs()
            .context("Cannot query output configs")?;

        // Prefer stereo float at the file's sample rate, fall back to defaults.
        let config = supported_configs
            .find(|c| {
                c.channels() == 2
                    && c.min_sample_rate().0 <= sample_rate
                    && c.max_sample_rate().0 >= sample_rate
                    && c.sample_format() == cpal::SampleFormat::F32
            })
            .map(|c| c.with_sample_rate(cpal::SampleRate(sample_rate)))
            .or_else(|| {
                device
                    .supported_output_configs()
                    .ok()?
                    .find(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
                    .map(|c| c.with_max_sample_rate())
            })
            .context("No compatible stereo f32 output config")?;

        let device_sample_rate = config.sample_rate().0;
        if device_sample_rate != sample_rate {
            tracing::warn!(
                file_rate = sample_rate,
                device_rate = device_sample_rate,
                "Sample rate mismatch — audio may sound off"
            );
        }

        let params = Arc::new(AudioParams::new(sample_rate));

        // Ring buffer: ~0.5s of stereo audio
        let ring_capacity = (device_sample_rate as usize) * 2;
        let rb = HeapRb::<f32>::new(ring_capacity);
        let (producer, mut consumer) = rb.split();

        // Spawn processing thread
        {
            let decoded = Arc::clone(&decoded);
            let params = Arc::clone(&params);
            std::thread::Builder::new()
                .name("screwdriver-proc".into())
                .spawn(move || processing_thread(decoded, params, producer, sample_rate))
                .context("Failed to spawn processing thread")?;
        }

        // cpal output stream — real-time safe: no alloc, no lock
        let params_rt = Arc::clone(&params);
        let stream = device.build_output_stream(
            &config.into(),
            move |output: &mut [f32], _| {
                if !params_rt.playing.load(Ordering::Relaxed) {
                    output.fill(0.0);
                    return;
                }
                let filled = consumer.pop_slice(output);
                output[filled..].fill(0.0);
                // Update position counter (stereo interleaved: divide by 2 for frame count)
                let prev = params_rt.position_samples.load(Ordering::Relaxed);
                params_rt
                    .position_samples
                    .store(prev + filled as u64, Ordering::Relaxed);
            },
            |err| tracing::error!("cpal stream error: {err}"),
            None,
        )?;

        stream.play()?;

        Ok(Self {
            params,
            device_sample_rate,
            _stream: stream,
        })
    }
}

fn processing_thread(
    decoded: Arc<Vec<f32>>,
    params: Arc<AudioParams>,
    mut producer: impl Producer<Item = f32>,
    sample_rate: u32,
) {
    let mut rb = RubberBand::new(sample_rate, 2, OPTION_PROCESS_REAL_TIME);

    // Prime rubberband with silence to fill its start pad
    let pad = rb.preferred_start_pad();
    if pad > 0 {
        let silence = vec![0.0f32; pad * 2];
        rb.process_interleaved(&silence, false);
    }

    // Drain and discard start delay samples
    let delay = rb.start_delay();
    let mut delay_remaining = delay * 2; // stereo

    let mut pos: usize = 0; // position in decoded (interleaved stereo samples)

    loop {
        if params.stop_flag.load(Ordering::Relaxed) {
            break;
        }

        // Handle seek
        let seek = params.seek_to.load(Ordering::Relaxed);
        if seek != u64::MAX {
            if params
                .seek_to
                .compare_exchange(seek, u64::MAX, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                pos = (seek as usize).min(decoded.len().saturating_sub(2));
                // Reset rubberband state
                rb = RubberBand::new(sample_rate, 2, OPTION_PROCESS_REAL_TIME);
                let pad = rb.preferred_start_pad();
                if pad > 0 {
                    let silence = vec![0.0f32; pad * 2];
                    rb.process_interleaved(&silence, false);
                }
                delay_remaining = rb.start_delay() * 2;
                params.position_samples.store(seek, Ordering::Relaxed);
            }
        }

        if !params.playing.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Loop handling
        let looping = params.looping.load(Ordering::Relaxed);
        let loop_in_s = params.loop_in.load(Ordering::Relaxed);
        let loop_out_s = params.loop_out.load(Ordering::Relaxed);
        if looping && loop_in_s >= 0.0 && loop_out_s > loop_in_s {
            let out_sample = (loop_out_s * sample_rate as f32 * 2.0) as usize;
            if pos >= out_sample {
                let in_sample = (loop_in_s * sample_rate as f32 * 2.0) as usize;
                pos = in_sample;
                rb = RubberBand::new(sample_rate, 2, OPTION_PROCESS_REAL_TIME);
                let pad = rb.preferred_start_pad();
                if pad > 0 {
                    let silence = vec![0.0f32; pad * 2];
                    rb.process_interleaved(&silence, false);
                }
                delay_remaining = rb.start_delay() * 2;
                params
                    .position_samples
                    .store(in_sample as u64, Ordering::Relaxed);
            }
        }

        if pos >= decoded.len() {
            // End of file — sleep and wait
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // Update rubberband params
        let pitch = params.pitch_semitones.load(Ordering::Relaxed);
        let tempo = params.tempo_multiplier.load(Ordering::Relaxed);
        rb.set_pitch_semitones(pitch);
        rb.set_tempo_multiplier(tempo);

        // Feed a chunk
        let end = (pos + CHUNK * 2).min(decoded.len());
        let chunk = &decoded[pos..end];
        let is_final = end >= decoded.len();
        rb.process_interleaved(chunk, is_final);
        pos = end;

        // Drain output
        loop {
            let avail = rb.available();
            if avail <= 0 {
                break;
            }
            let out = rb.retrieve_interleaved(avail as usize);

            // Discard startup delay
            if delay_remaining > 0 {
                let skip = delay_remaining.min(out.len());
                delay_remaining -= skip;
                if skip < out.len() {
                    producer.push_slice(&out[skip..]);
                }
            } else {
                producer.push_slice(&out);
            }
        }

        // Back-pressure: if ring nearly full, sleep briefly
        if producer.vacant_len() < CHUNK * 4 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
