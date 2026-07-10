//! Platform-neutral core logic for FadeWire.
//!
//! The signal-path math here (taper shapes, snap-band EMA, mute detent,
//! output hysteresis) is ported from — and stays behaviourally equivalent to —
//! the Windows sibling project
//! [zmk-volume-fader](https://github.com/Aleblazer/zmk-volume-fader)
//! (MIT, same author), where it was tuned against real slide-pot hardware.

pub mod calibration;
pub mod config;
pub mod filter;
pub mod mixer;
pub mod state;

/// The fader HID report (vendor page 0xFF00, report id 2) carries up to
/// eight signed 16-bit little-endian axes of raw wiper mV.
pub const MAX_AXES: usize = 8;

/// Convert an applied percentage (0..=100) to a PipeWire *native* channel
/// volume (linear amplitude). PipeWire's `Props` `channelVolumes` are linear,
/// while every user-facing mixer (pavucontrol, pactl %) works on a cubic
/// scale — so a fader percentage maps to amplitude as `(p/100)^3`.
///
/// Only use this when driving nodes through the native PipeWire API. When
/// going through the PulseAudio compatibility API, set the pulse volume as
/// `PA_VOLUME_NORM * p / 100` directly and the server applies the curve.
pub fn percent_to_channel_volume(percent: i32) -> f32 {
    let f = (percent.clamp(0, 100) as f32) / 100.0;
    f * f * f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_volume_is_cubic() {
        assert_eq!(percent_to_channel_volume(0), 0.0);
        assert_eq!(percent_to_channel_volume(100), 1.0);
        let half = percent_to_channel_volume(50);
        assert!((half - 0.125).abs() < 1e-6);
        // Clamped outside the range.
        assert_eq!(percent_to_channel_volume(-5), 0.0);
        assert_eq!(percent_to_channel_volume(250), 1.0);
    }
}
