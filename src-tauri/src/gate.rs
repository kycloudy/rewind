//! Noise gate for the microphone. It mutes the mic while its level stays below
//! a threshold, so keyboard, fan, and room noise stay out of clips.
//!
//! The gate opens fast (attack) when the level goes above the threshold. It
//! stays open for a short time after the level drops (hold), then fades out
//! (release). It closes only below a lower level (hysteresis), so a voice near
//! the threshold does not make it flutter.

const ATTACK_MS: f32 = 2.0;
const HOLD_MS: f32 = 200.0;
const RELEASE_MS: f32 = 150.0;
/// How fast the level detector falls after a peak.
const DETECT_MS: f32 = 10.0;
/// The gate closes this many dB below the threshold.
const HYSTERESIS_DB: f32 = 6.0;

pub struct Gate {
    open_level: f32,
    close_level: f32,
    hold_frames: u32,
    attack: f32,
    release: f32,
    detect: f32,
    level: f32,
    gain: f32,
    hold_left: u32,
    open: bool,
}

impl Gate {
    pub fn new(threshold_db: f32, rate: u32) -> Self {
        let rate = rate as f32;
        // One-pole smoothing coefficient for a time constant in milliseconds.
        let coef = |ms: f32| 1.0 - (-1000.0 / (ms * rate)).exp();
        Self {
            open_level: db_to_amp(threshold_db),
            close_level: db_to_amp(threshold_db - HYSTERESIS_DB),
            hold_frames: (HOLD_MS / 1000.0 * rate) as u32,
            attack: coef(ATTACK_MS),
            release: coef(RELEASE_MS),
            detect: 1.0 - coef(DETECT_MS),
            level: 0.0,
            gain: 0.0,
            hold_left: 0,
            open: false,
        }
    }

    /// Gates one stereo frame in place.
    pub fn process(&mut self, frame: &mut [f32]) {
        let peak = frame.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        self.level = if peak > self.level { peak } else { self.level * self.detect };

        if self.level >= self.open_level {
            self.open = true;
            self.hold_left = self.hold_frames;
        } else if self.open && self.level < self.close_level {
            if self.hold_left > 0 {
                self.hold_left -= 1;
            } else {
                self.open = false;
            }
        }

        let target = if self.open { 1.0 } else { 0.0 };
        let coef = if target > self.gain { self.attack } else { self.release };
        self.gain += (target - self.gain) * coef;
        for s in frame {
            *s *= self.gain;
        }
    }
}

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// Runs `ms` of a constant-amplitude signal through the gate and returns the last output sample.
    fn run(g: &mut Gate, amp: f32, ms: u32) -> f32 {
        let mut last = 0.0;
        for i in 0..(RATE * ms / 1000) {
            let v = if i % 2 == 0 { amp } else { -amp };
            let mut f = [v, v];
            g.process(&mut f);
            last = f[0].abs();
        }
        last
    }

    #[test]
    fn mutes_noise_below_threshold() {
        let mut g = Gate::new(-40.0, RATE);
        assert!(run(&mut g, db_to_amp(-50.0), 500) < 1e-6);
    }

    #[test]
    fn passes_signal_above_threshold() {
        let mut g = Gate::new(-40.0, RATE);
        let amp = db_to_amp(-20.0);
        assert!((run(&mut g, amp, 50) - amp).abs() < amp * 0.01);
    }

    #[test]
    fn holds_then_releases() {
        let mut g = Gate::new(-40.0, RATE);
        let amp = db_to_amp(-20.0);
        run(&mut g, amp, 50);
        // Still open during the hold time.
        let quiet = db_to_amp(-60.0);
        assert!((run(&mut g, quiet, 150) - quiet).abs() < quiet * 0.01);
        // Closed after hold plus release.
        assert!(run(&mut g, quiet, 1000) < quiet * 0.01);
    }

    #[test]
    fn stays_open_between_close_and_open_levels() {
        let mut g = Gate::new(-40.0, RATE);
        run(&mut g, db_to_amp(-20.0), 50);
        let amp = db_to_amp(-43.0);
        assert!((run(&mut g, amp, 1000) - amp).abs() < amp * 0.01);
    }
}
