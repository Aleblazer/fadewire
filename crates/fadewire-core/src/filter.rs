//! The per-axis signal path from a raw HID reading to an applied volume %.
//!
//! Pipeline (order matters, tuned on real hardware):
//! 1. **Snap-band EMA** — smooth ~±15 mV wiper noise, but *snap* on any jump
//!    beyond [`SNAP_BAND`]. Reports arrive per change (plus a slow heartbeat
//!    at rest), so easing a big jump would leave the smoothed value stranded
//!    when the reports stop: a fast pull to the end would creep the last few
//!    percent at heartbeat pace. Smoothing only ever filters idle flicker.
//! 2. **Taper curve** — raw -> fader % through the calibration curve.
//! 3. **Mute detent** — gated on the *instantaneous* raw reading (the
//!    smoothed value lags a slow pull), latched with [`MUTE_EXIT_BAND`] so
//!    boundary jitter can't flicker the mute.
//! 4. **Cap + hysteresis** — scale into `0..=cap`, then hold the previous
//!    integer % until the new value moves more than [`HYSTERESIS`] away, so
//!    a parked fader can't flip-flop between two percentages.

use crate::calibration::{eval, Calibration};

/// A raw jump beyond this (mV) is real movement, not noise — track it exactly.
/// Wiper noise is ~±15 mV; the firmware's rest band is 30 mV.
pub const SNAP_BAND: i32 = 60;

/// The mute dead zone unlatches only this far (mV) above its threshold.
pub const MUTE_EXIT_BAND: i32 = 15;

/// Output hysteresis (percent): hold the current % until the value moves more
/// than this off it. Must be < 1 so a full percent step always lands.
pub const HYSTERESIS: f64 = 0.9;

/// EMA weight kept by the previous smoothed value inside the snap band.
pub const EMA_KEEP: f64 = 0.85;

/// Per-axis filter state. Feed it raw readings; it returns the volume % to
/// apply. One instance per physical fader.
#[derive(Debug, Clone)]
pub struct AxisFilter {
    sm: f64,
    initialized: bool,
    last_raw: i32,
    in_mute_zone: bool,
    last_applied: i32,
}

impl Default for AxisFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl AxisFilter {
    pub fn new() -> Self {
        Self {
            sm: 0.0,
            initialized: false,
            last_raw: 0,
            in_mute_zone: false,
            last_applied: -1,
        }
    }

    /// Forget all state (device reconnected / recalibrated).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The most recent raw reading fed in.
    pub fn last_raw(&self) -> i32 {
        self.last_raw
    }

    /// The smoothed raw value (for UI readouts; drive volume via [`update`]).
    ///
    /// [`update`]: AxisFilter::update
    pub fn smoothed(&self) -> f64 {
        self.sm
    }

    /// Whether the mute detent is currently latched.
    pub fn in_mute_zone(&self) -> bool {
        self.in_mute_zone
    }

    /// Process one raw reading and return the volume % (0..=100) to apply,
    /// with `cap_percent` as the fader's maximum (its throw scales into
    /// `0..=cap`).
    pub fn update(
        &mut self,
        raw: i32,
        cal: &Calibration,
        curve: &[(i32, i32)],
        cap_percent: i32,
    ) -> i32 {
        self.last_raw = raw;

        // 1. Snap-band EMA.
        if !self.initialized || (raw as f64 - self.sm).abs() > SNAP_BAND as f64 {
            self.sm = raw as f64;
            self.initialized = true;
        } else {
            self.sm = self.sm * EMA_KEEP + raw as f64 * (1.0 - EMA_KEEP);
        }

        // 2. Taper curve.
        let mut fader_pct = eval(curve, self.sm.round() as i32);

        // 3. Mute detent, latched on the instantaneous reading.
        if cal.mute_raw > 0 {
            if raw < cal.mute_raw {
                self.in_mute_zone = true;
            } else if raw > cal.mute_raw + MUTE_EXIT_BAND {
                self.in_mute_zone = false;
            }
        } else {
            self.in_mute_zone = false;
        }
        if self.in_mute_zone {
            fader_pct = 0.0;
        }

        // 4. Cap + hysteresis.
        let pf = (fader_pct * cap_percent as f64 / 100.0).clamp(0.0, 100.0);
        let applied = if self.last_applied < 0 || (pf - self.last_applied as f64).abs() > HYSTERESIS
        {
            pf.round() as i32
        } else {
            self.last_applied
        };
        self.last_applied = applied;
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::Taper;

    fn straight() -> Calibration {
        Calibration {
            min: 0,
            max: 3300,
            taper: Taper::Straight,
            mute_raw: 0,
        }
    }

    #[test]
    fn fast_pull_lands_exactly_no_creep() {
        let cal = straight();
        let curve = cal.build_curve();
        let mut f = AxisFilter::new();
        assert_eq!(f.update(3300, &cal, &curve, 100), 100);
        // One report at the far end: the snap band tracks it exactly instead
        // of easing toward it (the creep bug this design killed).
        assert_eq!(f.update(0, &cal, &curve, 100), 0);
    }

    #[test]
    fn noise_inside_the_band_is_smoothed_and_held() {
        let cal = straight();
        let curve = cal.build_curve();
        let mut f = AxisFilter::new();
        assert_eq!(f.update(1650, &cal, &curve, 100), 50);
        // +40 mV is inside the snap band: EMA moves a little, hysteresis
        // holds the applied % completely still.
        assert_eq!(f.update(1690, &cal, &curve, 100), 50);
        assert!(f.smoothed() > 1650.0 && f.smoothed() < 1690.0);
    }

    #[test]
    fn mute_detent_latches_and_needs_the_exit_band() {
        let cal = Calibration {
            mute_raw: 10,
            ..straight()
        };
        let curve = cal.build_curve();
        let mut f = AxisFilter::new();
        assert_eq!(f.update(5, &cal, &curve, 100), 0);
        assert!(f.in_mute_zone());
        // Above the threshold but inside the exit band: still latched.
        assert_eq!(f.update(20, &cal, &curve, 100), 0);
        assert!(f.in_mute_zone());
        // Clearly above threshold + band: unlatched.
        f.update(26, &cal, &curve, 100);
        assert!(!f.in_mute_zone());
        // Real movement out of the detent tracks immediately (snap band).
        assert_eq!(f.update(1650, &cal, &curve, 100), 50);
    }

    #[test]
    fn mute_detent_is_instant_even_when_the_ema_lags() {
        // Slow descent: the smoothed value trails the raw reading, but the
        // detent gates on the instantaneous raw and must mute NOW.
        let cal = Calibration {
            mute_raw: 10,
            ..straight()
        };
        let curve = cal.build_curve();
        let mut f = AxisFilter::new();
        f.update(60, &cal, &curve, 100);
        f.update(30, &cal, &curve, 100); // inside snap band, EMA lags above 10
        let applied = f.update(5, &cal, &curve, 100);
        assert!(f.smoothed() > 10.0, "test premise: EMA still above threshold");
        assert_eq!(applied, 0);
    }

    #[test]
    fn cap_scales_the_throw() {
        let cal = straight();
        let curve = cal.build_curve();
        let mut f = AxisFilter::new();
        // Top of travel with a 60% cap = 60%; middle = 30%.
        assert_eq!(f.update(3300, &cal, &curve, 60), 60);
        assert_eq!(f.update(1650, &cal, &curve, 60), 30);
    }

    #[test]
    fn hysteresis_never_blocks_a_full_step() {
        let cal = straight();
        let curve = cal.build_curve();
        let mut f = AxisFilter::new();
        assert_eq!(f.update(1650, &cal, &curve, 100), 50);
        // 1683 mV = 51.0% — exactly one percent away. The step is inside the
        // snap band, so let the EMA converge; the hysteresis (0.9) must then
        // release the held 50 and land on 51.
        let mut last = 50;
        for _ in 0..40 {
            last = f.update(1683, &cal, &curve, 100);
        }
        assert_eq!(last, 51);
    }
}
