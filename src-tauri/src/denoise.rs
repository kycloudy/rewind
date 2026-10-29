//! Noise suppression for the microphone, with RNNoise (the `nnnoiseless` port).
//! It removes steady noise such as fans, hum, and hiss, and keeps the voice.
//!
//! RNNoise works on mono 48 kHz audio in blocks of 480 samples (10 ms). The
//! mixer gives us frames in any count, so we queue input until a block is full.
//! The output runs one block behind the input, which adds 10 ms of mic delay.

use nnnoiseless::DenoiseState;
use std::collections::VecDeque;

const BLOCK: usize = DenoiseState::FRAME_SIZE;
/// RNNoise expects samples in 16-bit range.
const SCALE: f32 = 32768.0;

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    input: Vec<f32>,
    output: VecDeque<f32>,
    block_out: Vec<f32>,
    /// The first block out of RNNoise has a fade-in artifact, so we drop it.
    first: bool,
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            input: Vec::with_capacity(BLOCK),
            output: VecDeque::from(vec![0.0; BLOCK]),
            block_out: vec![0.0; BLOCK],
            first: true,
        }
    }

    /// Denoises one stereo frame in place. The output is mono on both channels.
    pub fn process(&mut self, frame: &mut [f32; 2]) {
        self.input.push((frame[0] + frame[1]) * 0.5 * SCALE);
        if self.input.len() == BLOCK {
            self.state.process_frame(&mut self.block_out, &self.input);
            self.input.clear();
            let first = std::mem::take(&mut self.first);
            self.output.extend(self.block_out.iter().map(|s| if first { 0.0 } else { s / SCALE }));
        }
        let s = self.output.pop_front().unwrap_or(0.0);
        *frame = [s, s];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Repeatable low-pass noise, similar to a fan or room hum.
    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let (mut seed, mut y) = (1u32, 0.0f32);
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                y += (white - y) * 0.05;
                y * amp
            })
            .collect()
    }

    fn run(d: &mut Denoiser, input: &[f32]) -> Vec<f32> {
        input
            .iter()
            .map(|&s| {
                let mut f = [s, s];
                d.process(&mut f);
                f[0]
            })
            .collect()
    }

    #[test]
    fn reduces_steady_noise() {
        let mut d = Denoiser::new();
        let input = noise(48_000 * 2, 0.3);
        let out = run(&mut d, &input);
        // Compare the last second, after the model settles.
        let tail = 48_000..;
        assert!(rms(&out[tail.clone()]) < rms(&input[tail]) * 0.1);
    }

    #[test]
    fn output_length_matches_input() {
        let mut d = Denoiser::new();
        assert_eq!(run(&mut d, &noise(1234, 0.01)).len(), 1234);
    }
}
