//! Per-fader calibration: the captured raw range plus the taper preset that
//! maps a raw reading to a volume percentage between the ends.
//!
//! The taper presets are the inverse of the Bourns slide-pot output-vs-travel
//! curves (value-fraction % -> volume %), so physical travel maps roughly
//! linearly onto volume even though the pot's electrical output is compressed
//! near the top of its throw.

use serde::{Deserialize, Serialize};

/// Which taper preset maps the raw value to a percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Taper {
    /// A "linear" (B-curve) pot, corrected for its compressed top of travel.
    #[default]
    Linear,
    /// An audio (A-curve) pot.
    Audio,
    /// No correction: raw maps straight to percent.
    Straight,
}

/// A fader's captured range, taper, and mute dead zone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    /// Raw reading at the bottom of travel (mV).
    pub min: i32,
    /// Raw reading at the top of travel (mV).
    pub max: i32,
    pub taper: Taper,
    /// Mute dead zone (raw mV): while the *instantaneous* raw reading sits
    /// below this, output is forced to 0% — a mixer-style mute detent at the
    /// bottom of the throw. Covers a wiper that rests a few mV above the
    /// calibrated `min` and would otherwise hover at ~1%. `0` = off.
    pub mute_raw: i32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            min: 4,
            max: 3215,
            taper: Taper::Linear,
            mute_raw: 0,
        }
    }
}

// Presets as (value-fraction %, volume %) control points.
const LINEAR_SHAPE: &[(f64, f64)] = &[
    (0.0, 0.0),
    (1.0, 10.0),
    (4.0, 20.0),
    (10.0, 30.0),
    (30.0, 40.0),
    (50.0, 50.0),
    (63.0, 60.0),
    (78.0, 70.0),
    (92.0, 80.0),
    (98.0, 90.0),
    (100.0, 100.0),
];
const AUDIO_SHAPE: &[(f64, f64)] = &[
    (0.0, 0.0),
    (1.0, 10.0),
    (3.0, 20.0),
    (6.0, 30.0),
    (10.0, 40.0),
    (15.0, 50.0),
    (22.0, 60.0),
    (38.0, 70.0),
    (65.0, 80.0),
    (90.0, 90.0),
    (100.0, 100.0),
];
const STRAIGHT_SHAPE: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 100.0)];

impl Calibration {
    /// Build the piecewise (raw -> %) lookup table that [`eval`] interpolates.
    pub fn build_curve(&self) -> Vec<(i32, i32)> {
        let (mut lo, mut hi) = (self.min.min(self.max), self.min.max(self.max));
        if hi - lo < 2 {
            // Degenerate range (nothing captured yet): fall back to full scale.
            lo = 0;
            hi = 3250;
        }
        let shape = match self.taper {
            Taper::Audio => AUDIO_SHAPE,
            Taper::Straight => STRAIGHT_SHAPE,
            Taper::Linear => LINEAR_SHAPE,
        };
        let mut out: Vec<(i32, i32)> = shape
            .iter()
            .map(|&(f, v)| {
                (
                    (lo as f64 + f / 100.0 * (hi - lo) as f64).round() as i32,
                    v.round() as i32,
                )
            })
            .collect();
        // Guarantee strictly-increasing raw values so interpolation stays
        // well-defined even on a tiny captured range.
        for i in 1..out.len() {
            if out[i].0 <= out[i - 1].0 {
                out[i].0 = out[i - 1].0 + 1;
            }
        }
        out
    }
}

/// Piecewise-linear lookup; clamps to the end points (continuous dead bands
/// past either end of the captured range).
pub fn eval(curve: &[(i32, i32)], v: i32) -> f64 {
    let Some(&(v0, p0)) = curve.first() else {
        return 0.0;
    };
    if v <= v0 {
        return p0 as f64;
    }
    for i in 1..curve.len() {
        if v <= curve[i].0 {
            let (va, pa) = curve[i - 1];
            let (vb, pb) = curve[i];
            return pa as f64 + (v - va) as f64 / (vb - va) as f64 * (pb - pa) as f64;
        }
    }
    curve[curve.len() - 1].1 as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_taper_maps_linearly() {
        let cal = Calibration {
            min: 0,
            max: 3300,
            taper: Taper::Straight,
            mute_raw: 0,
        };
        let curve = cal.build_curve();
        assert_eq!(eval(&curve, 0), 0.0);
        assert_eq!(eval(&curve, 3300), 100.0);
        assert!((eval(&curve, 1650) - 50.0).abs() < 0.1);
    }

    #[test]
    fn eval_clamps_past_the_ends() {
        let curve = Calibration::default().build_curve();
        assert_eq!(eval(&curve, -100), 0.0);
        assert_eq!(eval(&curve, 0), 0.0);
        assert_eq!(eval(&curve, 100_000), 100.0);
    }

    #[test]
    fn degenerate_range_falls_back_to_full_scale() {
        let cal = Calibration {
            min: 500,
            max: 501,
            taper: Taper::Straight,
            mute_raw: 0,
        };
        let curve = cal.build_curve();
        assert_eq!(curve.first().unwrap().0, 0);
        assert_eq!(curve.last().unwrap().0, 3250);
    }

    #[test]
    fn curve_raw_values_strictly_increase() {
        // A tiny (but non-degenerate) range would collapse control points
        // without the sanitize pass.
        let cal = Calibration {
            min: 0,
            max: 5,
            taper: Taper::Linear,
            mute_raw: 0,
        };
        let curve = cal.build_curve();
        for w in curve.windows(2) {
            assert!(w[1].0 > w[0].0, "non-increasing: {:?}", curve);
        }
    }

    #[test]
    fn linear_taper_boosts_the_bottom_of_travel() {
        // The Linear preset maps the first 1% of the raw range to 10% volume
        // (inverse of the pot's compressed output near the ends).
        let cal = Calibration {
            min: 0,
            max: 3300,
            taper: Taper::Linear,
            mute_raw: 0,
        };
        let curve = cal.build_curve();
        assert!((eval(&curve, 33) - 10.0).abs() < 0.5);
    }
}
