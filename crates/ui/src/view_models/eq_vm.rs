//! Turning the equaliser into something that can be drawn.
//!
//! Two coordinate systems meet here. The listener's is hertz and decibels; the
//! screen's is a fraction across and a fraction down. Every conversion between
//! them lives in this file, so a control that reads wrong reads wrong in one
//! place.

use cadenza_core::domain::eq::{EqMode, EqPreset, EqSetting};
use cadenza_core::domain::value_objects::gain::{MAX_GAIN_DB, MIN_GAIN_DB};

use crate::{EqBandData, EqPresetData};

/// Where a gain sits down the screen, from 0 at the top.
pub fn gain_to_y(gain_db: f32) -> f32 {
    ((MAX_GAIN_DB - gain_db) / (MAX_GAIN_DB - MIN_GAIN_DB)).clamp(0.0, 1.0)
}

/// The gain at a fraction down the screen.
pub fn y_to_gain(y: f32) -> f32 {
    MAX_GAIN_DB - y.clamp(0.0, 1.0) * (MAX_GAIN_DB - MIN_GAIN_DB)
}

/// The bells, as the screen places them.
pub fn bands(setting: &EqSetting, selected: usize) -> Vec<EqBandData> {
    setting
        .advanced
        .iter()
        .enumerate()
        .map(|(index, band)| EqBandData {
            index: index as i32,
            y: gain_to_y(band.gain().as_db()),
            frequency: hertz(band.frequency_hz()).into(),
            q: format!("Q {:.1}", band.q()).into(),
            gain: decibels(band.gain().as_db()).into(),
            selected: index == selected,
        })
        .collect()
}

/// The presets, built-in and the listener's own.
pub fn presets(all: &[EqPreset], setting: &EqSetting) -> Vec<EqPresetData> {
    all.iter()
        .map(|preset| EqPresetData {
            id: preset.id.to_string().into(),
            name: preset.name.to_uppercase().into(),
            // Lit when what is playing is exactly this preset — in the mode
            // that is in use, because that is the half being heard. Nudge one
            // control and nothing is lit, which is the truth: the sound is no
            // longer any preset at all.
            selected: match setting.mode {
                EqMode::Simple => preset.simple == setting.simple,
                EqMode::Advanced => preset.advanced == setting.advanced,
            },
            editable: !preset.is_builtin,
        })
        .collect()
}

/// The line under the heading: which mode, and whether anything is being done.
pub fn summary_line(setting: &EqSetting) -> String {
    let mode = match setting.mode {
        EqMode::Simple => "three controls",
        EqMode::Advanced => "eight bands",
    };
    if setting.is_flat() {
        format!("{mode} · flat")
    } else {
        format!("{mode} · in use")
    }
}

/// A gain, with its sign, the way a control reads it out.
pub fn decibels(db: f32) -> String {
    if db.abs() < 0.05 {
        "0".to_owned()
    } else {
        format!("{db:+.1}")
    }
}

/// A frequency, short enough to sit under a node.
pub fn hertz(frequency_hz: u32) -> String {
    if frequency_hz >= 1_000 {
        let thousands = frequency_hz as f32 / 1_000.0;
        if (thousands - thousands.round()).abs() < 0.05 {
            format!("{:.0}k", thousands)
        } else {
            format!("{thousands:.1}k")
        }
    } else {
        format!("{frequency_hz}")
    }
}

#[cfg(test)]
mod tests {
    use super::{decibels, gain_to_y, hertz, y_to_gain};

    #[test]
    fn the_top_of_the_screen_is_the_loudest_a_band_goes() {
        assert_eq!(gain_to_y(12.0), 0.0);
        assert_eq!(gain_to_y(-12.0), 1.0);
        assert!((gain_to_y(0.0) - 0.5).abs() < f32::EPSILON);
        assert!((y_to_gain(0.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn a_gain_reads_out_with_its_sign_and_a_frequency_short() {
        assert_eq!(decibels(0.0), "0");
        assert_eq!(decibels(4.0), "+4.0");
        assert_eq!(decibels(-7.5), "-7.5");

        assert_eq!(hertz(60), "60");
        assert_eq!(hertz(1_000), "1k");
        assert_eq!(hertz(2_500), "2.5k");
        assert_eq!(hertz(16_000), "16k");
    }
}
