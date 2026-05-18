use std::ffi::c_void;

pub const OPTION_PROCESS_REAL_TIME: u32 = 0x00000001;
pub const OPTION_PROCESS_OFFLINE: u32 = 0x00000000;

type RubberBandState = *mut c_void;

#[allow(non_snake_case)]
extern "C" {
    fn rubberband_new(
        sample_rate: u32,
        channels: u32,
        options: u32,
        initial_time_ratio: f64,
        initial_pitch_scale: f64,
    ) -> RubberBandState;
    fn rubberband_delete(state: RubberBandState);
    fn rubberband_set_time_ratio(state: RubberBandState, ratio: f64);
    fn rubberband_set_pitch_scale(state: RubberBandState, scale: f64);
    fn rubberband_process(
        state: RubberBandState,
        input: *const *const f32,
        samples: usize,
        is_final: i32,
    );
    fn rubberband_available(state: RubberBandState) -> i32;
    fn rubberband_retrieve(
        state: RubberBandState,
        output: *const *mut f32,
        samples: usize,
    ) -> usize;
    fn rubberband_get_preferred_start_pad(state: RubberBandState) -> usize;
    fn rubberband_get_start_delay(state: RubberBandState) -> usize;
}

pub struct RubberBand {
    state: RubberBandState,
    channels: usize,
}

// Safe to send to processing thread — we only ever use it from one thread.
unsafe impl Send for RubberBand {}

impl RubberBand {
    pub fn new(sample_rate: u32, channels: u32, options: u32) -> Self {
        let state = unsafe { rubberband_new(sample_rate, channels, options, 1.0, 1.0) };
        assert!(!state.is_null(), "rubberband_new returned null");
        Self {
            state,
            channels: channels as usize,
        }
    }

    pub fn set_pitch_semitones(&self, semitones: f32) {
        let scale = 2f64.powf(semitones as f64 / 12.0);
        unsafe { rubberband_set_pitch_scale(self.state, scale) };
    }

    pub fn set_tempo_multiplier(&self, tempo: f32) {
        let ratio = 1.0 / (tempo as f64).max(0.01);
        unsafe { rubberband_set_time_ratio(self.state, ratio) };
    }

    pub fn start_delay(&self) -> usize {
        unsafe { rubberband_get_start_delay(self.state) }
    }

    pub fn preferred_start_pad(&self) -> usize {
        unsafe { rubberband_get_preferred_start_pad(self.state) }
    }

    /// Process interleaved stereo input. De-interleaves, feeds rubberband, does NOT retrieve.
    pub fn process_interleaved(&self, interleaved: &[f32], is_final: bool) {
        let frames = interleaved.len() / self.channels;
        // De-interleave into per-channel vecs
        let mut channels: Vec<Vec<f32>> =
            (0..self.channels).map(|_| vec![0.0f32; frames]).collect();
        for (i, &s) in interleaved.iter().enumerate() {
            channels[i % self.channels][i / self.channels] = s;
        }
        let ptrs: Vec<*const f32> = channels.iter().map(|c| c.as_ptr()).collect();
        unsafe {
            rubberband_process(
                self.state,
                ptrs.as_ptr(),
                frames,
                if is_final { 1 } else { 0 },
            );
        }
    }

    pub fn available(&self) -> i32 {
        unsafe { rubberband_available(self.state) }
    }

    /// Retrieve up to `count` frames and return as interleaved stereo Vec<f32>.
    pub fn retrieve_interleaved(&self, count: usize) -> Vec<f32> {
        let mut channels: Vec<Vec<f32>> = (0..self.channels).map(|_| vec![0.0f32; count]).collect();
        let ptrs: Vec<*mut f32> = channels.iter_mut().map(|c| c.as_mut_ptr()).collect();
        let retrieved = unsafe { rubberband_retrieve(self.state, ptrs.as_ptr(), count) };
        // Re-interleave
        let mut out = Vec::with_capacity(retrieved * self.channels);
        for frame in 0..retrieved {
            for ch in 0..self.channels {
                out.push(channels[ch][frame]);
            }
        }
        out
    }
}

impl Drop for RubberBand {
    fn drop(&mut self) {
        unsafe { rubberband_delete(self.state) };
    }
}
