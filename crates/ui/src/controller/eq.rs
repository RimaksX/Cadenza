//! The equaliser.

use super::*;

impl Controller {
    /// Everything the equaliser screen draws, presets included.
    pub fn refresh_eq(&self) {
        self.refresh_eq_controls();

        let (Some(window), Ok(setting), Ok(presets)) = (
            self.window.upgrade(),
            self.services.eq.current(),
            self.services.eq.list(),
        ) else {
            return;
        };

        let rows = eq_vm::presets(&presets, &setting);
        window
            .global::<Equaliser>()
            .set_presets(ModelRc::new(VecModel::from(rows)));
    }

    /// The controls alone: the dial, the curve, and what they read out.
    ///
    /// Separate from the presets because this runs on every step of a drag, and
    /// listing nine presets to find out that none of them is lit is a database
    /// query per pointer event.
    pub(super) fn refresh_eq_controls(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let eq = window.global::<Equaliser>();
        let Ok(setting) = self.services.eq.current() else {
            return;
        };

        let selected = self
            .selected_band
            .get()
            .min(setting.advanced.len().saturating_sub(1));
        eq.set_advanced(setting.mode == EqMode::Advanced);
        eq.set_bass(setting.simple.bass.as_db());
        eq.set_mid(setting.simple.mid.as_db());
        eq.set_treble(setting.simple.treble.as_db());
        eq.set_summary(eq_vm::summary_line(&setting).into());
        eq.set_selected(selected as i32);

        if let Some(band) = setting.advanced.get(selected) {
            eq.set_selected_frequency(eq_vm::hertz(band.frequency_hz()).into());
            eq.set_selected_q(format!("{:.1}", band.q()).into());
            eq.set_selected_gain(eq_vm::decibels(band.gain().as_db()).into());
        }

        // Written into the model that is already there, row by row. The first
        // pass fills it; from then on the elements the window built stay put,
        // which is what lets a fader be dragged rather than only clicked.
        let bands = eq_vm::bands(&setting, selected);
        if self.eq_bands.row_count() == bands.len() {
            for (index, band) in bands.into_iter().enumerate() {
                self.eq_bands.set_row_data(index, band);
            }
        } else {
            while self.eq_bands.row_count() > 0 {
                self.eq_bands.remove(0);
            }
            for band in bands {
                self.eq_bands.push(band);
            }
            eq.set_bands(ModelRc::from(Rc::clone(&self.eq_bands)));
        }
    }

    /// Switches between the three controls and the eight bells.
    pub fn set_eq_mode(&self, advanced: bool) {
        let mode = if advanced {
            EqMode::Advanced
        } else {
            EqMode::Simple
        };
        self.run(|| self.services.eq.set_mode(mode));
        self.refresh_eq();
    }

    /// Moves one of the three tone controls.
    ///
    /// The sound follows the hand and the database waits: a control being
    /// dragged reports every step of the way, and twenty-eight rows written per
    /// step is a database asked to keep up with a wrist.
    pub fn set_eq_simple(&self, which: ToneBand, decibels: f32) {
        self.run(|| {
            let mut setting = self.services.eq.current()?;
            let gain = GainDb::clamped(decibels);
            // Named, not numbered. While this was an `i32` the wildcard sent
            // every index that was not zero or one to treble, so a fourth
            // control added to the dial would have silently moved the third.
            match which {
                ToneBand::Bass => setting.simple.bass = gain,
                ToneBand::Mid => setting.simple.mid = gain,
                ToneBand::Treble => setting.simple.treble = gain,
            }
            setting.mode = EqMode::Simple;
            self.services.eq.preview(setting)
        });
        self.refresh_eq_controls();
    }

    /// Sets one band's gain from where its fader was left.
    pub fn move_eq_band(&self, index: i32, y: f32) {
        let index = index.max(0) as usize;
        self.selected_band.set(index);

        self.run(|| {
            let mut setting = self.services.eq.current()?;
            let band = *setting
                .advanced
                .get(index)
                .ok_or_else(|| CoreError::not_found("eq band", index))?;

            setting.advanced[index] = band.with_gain(GainDb::clamped(eq_vm::y_to_gain(y)));
            setting.mode = EqMode::Advanced;
            self.services.eq.preview(setting)
        });
        self.refresh_eq_controls();
    }

    /// The hand let go of a control: write down what it left behind.
    pub fn settle_eq(&self) {
        self.run(|| self.services.eq.commit());
        self.refresh_eq();
    }

    /// Says which bell the numbers under the curve are about.
    pub fn select_eq_band(&self, index: i32) {
        self.selected_band.set(index.max(0) as usize);
        self.refresh_eq();
    }

    /// Moves the chosen band along the spectrum, a sixth of an octave at a time.
    ///
    /// Buttons rather than a drag: a row of faders says nothing about where a
    /// band sits, so the frequency needs a control of its own — and a step that
    /// is a fraction of an octave moves by the same *musical* amount wherever
    /// the band happens to be.
    pub fn tune_eq_band(&self, index: i32, direction: i32) {
        let index = index.max(0) as usize;

        self.run(|| {
            let setting = self.services.eq.current()?;
            let band = setting
                .advanced
                .get(index)
                .ok_or_else(|| CoreError::not_found("eq band", index))?;

            let moved = f64::from(band.frequency_hz()) * 2.0_f64.powf(f64::from(direction) / 6.0);
            let frequency_hz = (moved.round() as u32).clamp(MIN_BAND_HZ, MAX_BAND_HZ);

            self.services
                .eq
                .set_band(index, EqBand::new(frequency_hz, band.q(), band.gain())?)
        });
        self.refresh_eq();
    }

    /// Widens or narrows a bell without moving it.
    pub fn widen_eq_band(&self, index: i32, step: f32) {
        let index = index.max(0) as usize;

        self.run(|| {
            let setting = self.services.eq.current()?;
            let band = setting
                .advanced
                .get(index)
                .ok_or_else(|| CoreError::not_found("eq band", index))?;

            // Rounded to the tenth the readout shows. Left at full precision
            // a step out and back lands on 0.99999994 rather than 1, and a
            // preset that had been chosen would stop matching itself over a
            // difference nobody can hear or see.
            let q = (((band.q() + step) * 10.0).round() / 10.0).clamp(MIN_BAND_Q, MAX_BAND_Q);

            self.services
                .eq
                .set_band(index, EqBand::new(band.frequency_hz(), q, band.gain())?)
        });
        self.refresh_eq();
    }

    /// Sets everything at once from a preset.
    pub fn pick_eq_preset(&self, id: &str) {
        let Ok(id) = EqPresetId::parse(id) else {
            return;
        };
        self.run(|| self.services.eq.apply_preset(id));
        self.refresh_eq();
    }

    /// Saves what is set now under a name of the listener's own.
    pub fn save_eq_preset(&self, name: &str) {
        self.run(|| self.services.eq.save_as(name).map(|_| ()));
        self.refresh_eq();
    }

    /// Gives one of the listener's own sounds another name.
    pub fn rename_eq_preset(&self, id: &str, name: &str) {
        let Ok(id) = EqPresetId::parse(id) else {
            return;
        };
        self.run(|| self.services.eq.rename(id, name));
        self.refresh_eq();
    }

    /// Throws one away. What is playing is unchanged.
    pub fn delete_eq_preset(&self, id: &str) {
        let Ok(id) = EqPresetId::parse(id) else {
            return;
        };
        self.run(|| self.services.eq.delete(id));
        self.refresh_eq();
    }

    /// Puts the equaliser back to doing nothing.
    pub fn reset_eq(&self) {
        self.run(|| self.services.eq.reset());
        self.refresh_eq();
    }
}
