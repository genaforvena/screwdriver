use crate::audio::rubberband::{RubberBand, OPTION_PROCESS_OFFLINE};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub struct Clip {
    pub source: PathBuf,
    pub in_point: f32,
    pub out_point: f32,
    pub pitch: f32,
    pub tempo: f32,
}

/// Render the region [clip.in_point, clip.out_point] of `decoded` through rubberband
/// (offline mode, best quality) and write the result as a 32-bit float WAV.
pub fn save_clip(clip: &Clip, decoded: &[f32], sample_rate: u32, path: &Path) -> Result<()> {
    // Convert time → interleaved stereo sample indices (2 samples per frame).
    let in_sample = ((clip.in_point * sample_rate as f32) as usize * 2).min(decoded.len());
    let out_sample = ((clip.out_point * sample_rate as f32) as usize * 2).min(decoded.len());

    anyhow::ensure!(in_sample < out_sample, "in point must be before out point");

    let region = &decoded[in_sample..out_sample];

    let rb = RubberBand::new(sample_rate, 2, OPTION_PROCESS_OFFLINE);
    rb.set_pitch_semitones(clip.pitch);
    rb.set_tempo_multiplier(clip.tempo);

    // Offline mode requires a study pass before processing.
    rb.study_interleaved(region, true);
    rb.process_interleaved(region, true);

    let mut output: Vec<f32> = Vec::with_capacity(region.len());
    loop {
        let avail = rb.available();
        if avail <= 0 {
            break;
        }
        let chunk = rb.retrieve_interleaved(avail as usize);
        output.extend_from_slice(&chunk);
    }

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in &output {
        writer.write_sample(s)?;
    }
    writer.finalize()?;

    tracing::info!(
        path = %path.display(),
        in_sec = clip.in_point,
        out_sec = clip.out_point,
        pitch_st = clip.pitch,
        tempo = clip.tempo,
        output_frames = output.len() / 2,
        "clip saved"
    );

    Ok(())
}
