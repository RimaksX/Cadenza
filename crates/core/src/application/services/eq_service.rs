//! The equaliser: what it is set to, and the presets it can be set from.
//!
//! Two things live here that look like one. A **preset** is a named curve in a
//! table; the **setting** is what the filters are actually doing. They part
//! company the moment somebody picks a preset and nudges a control, which is
//! most of the time — so the setting is stored in its own right rather than as
//! a pointer at a preset that no longer describes it (PROJECT_MASTER 2.8,
//! "сохранение состояния").
//!
//! And a third thing, which is neither: **the preset a track is played with.**
//! An equaliser set once for everything is set wrong for almost everything —
//! the curve that rescues a thin recording ruins a well-made one. So a preset
//! chosen while something is playing is remembered for that track, and every
//! track that has no choice of its own starts at Standard: what a listener did
//! to one record does not follow them into the next (`MASTER_ISSUES` 89).

use std::sync::{Arc, RwLock};

use crate::application::context::AppContext;
use crate::domain::eq::{EqBand, EqMode, EqPreset, EqSetting, SimpleEq};
use crate::domain::ids::{EqPresetId, MediaFileId, ProfileId};
use crate::domain::policies::eq_policy::{ADVANCED_BAND_COUNT, default_advanced_bands};
use crate::domain::ports::audio_engine::AudioEnginePort;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::repositories::{EqPresetRepositoryPort, TrackEqRepositoryPort};
use crate::domain::settings::SettingValue;
use crate::domain::value_objects::GainDb;
use crate::{CoreError, Result};

/// Where the mode is kept.
const MODE_KEY: &str = "eq.mode";

/// Where the three tone controls are kept.
const SIMPLE_KEYS: [&str; 3] = ["eq.simple.bass", "eq.simple.mid", "eq.simple.treble"];

/// Everything the equaliser talks to.
pub struct EqPorts {
    /// The presets table.
    pub presets: Arc<dyn EqPresetRepositoryPort>,
    /// What each track is to be played with.
    pub choices: Arc<dyn TrackEqRepositoryPort>,
    /// The filters themselves.
    pub engine: Arc<dyn AudioEnginePort>,
}

/// The equaliser's settings and presets.
pub struct EqService {
    context: Arc<AppContext>,
    ports: EqPorts,
    /// What is in force, and whose it is.
    ///
    /// Cached because the screen asks for it while it draws, and because a
    /// parametric setting is twenty-eight rows to read. The profile travels
    /// with it so a switch cannot be answered from the last listener's sound.
    current: RwLock<Option<(ProfileId, EqSetting)>>,
    /// What is playing, as [`Self::follow`] was last told.
    ///
    /// Held here rather than asked for, because this is the only thing the
    /// equaliser wants to know about the transport and asking would make the
    /// two services depend on each other in both directions.
    playing: RwLock<Option<MediaFileId>>,
}

impl EqService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: EqPorts) -> Self {
        Self {
            context,
            ports,
            current: RwLock::new(None),
            playing: RwLock::new(None),
        }
    }

    /// The built-in presets and the listener's own.
    pub fn list(&self) -> Result<Vec<EqPreset>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.presets.list_for_profile(profile_id)
    }

    /// What the filters are set to.
    ///
    /// Reading it is also what applies it: the engine has no database, so the
    /// first ask after a start is what puts the listener's sound back.
    pub fn current(&self) -> Result<EqSetting> {
        let profile_id = self.context.require_active_profile()?;

        if let Some((cached_for, setting)) = self
            .current
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            && *cached_for == profile_id
        {
            return Ok(setting.clone());
        }

        let setting = self.read_setting(profile_id)?;
        self.ports.engine.set_eq(&setting)?;
        *self.current.write().unwrap_or_else(|err| err.into_inner()) =
            Some((profile_id, setting.clone()));
        Ok(setting)
    }

    /// Sets everything at once, from a preset.
    ///
    /// **The listener stays in the mode they are in.** A preset describes the
    /// same intention twice — as three controls and as eight bands — so
    /// choosing one in the simple mode moves the three arms, and choosing it in
    /// the advanced mode moves the eight faders. Switching modes underneath
    /// somebody who pressed a preset was the first thing anybody noticed about
    /// this screen, and it was the screen being wrong rather than them.
    /// **And the track keeps it.** Choosing a preset while something is
    /// playing is how a listener says what that record should sound like, so
    /// it is written down against the file and applied again the next time it
    /// comes round. With nothing playing there is nothing to write it against,
    /// and the choice is simply the sound until something else changes it.
    pub fn apply_preset(&self, id: EqPresetId) -> Result<()> {
        self.set_from_preset(id)?;

        let profile_id = self.context.require_active_profile()?;
        if let Some(track) = *self.playing.read().unwrap_or_else(|err| err.into_inner()) {
            self.ports
                .choices
                .remember(profile_id, track, id, self.context.clock.now())?;
        }

        Ok(())
    }

    /// Puts the equaliser where the track that is starting wants it.
    ///
    /// Called by whatever opens a track, which is the queue. A track nobody has
    /// chosen for gets Standard rather than the last track's curve: the sound
    /// somebody set for one record is about that record, and carrying it into
    /// the next is how an equaliser ends up quietly ruining a library.
    ///
    /// The mode is left alone, the way [`Self::apply_preset`] leaves it: a
    /// preset describes the same intention as three controls and as eight
    /// bands, and the listener stays on the screen they were looking at.
    pub fn follow(&self, track: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        *self.playing.write().unwrap_or_else(|err| err.into_inner()) = Some(track);

        match self.ports.choices.preset_for(profile_id, track)? {
            Some(chosen) => self.set_from_preset(chosen),
            None => {
                let mut setting = EqSetting::flat();
                setting.mode = self.current()?.mode;
                self.write(setting)
            }
        }
    }

    /// Applies a preset without recording that anybody chose it.
    fn set_from_preset(&self, id: EqPresetId) -> Result<()> {
        let preset = self
            .ports
            .presets
            .get(id)?
            .ok_or_else(|| CoreError::not_found("eq preset", id))?;

        if let Some(owner) = preset.profile_id
            && owner != self.context.require_active_profile()?
        {
            return Err(CoreError::not_found("eq preset", id));
        }

        let mut setting = EqSetting::from(&preset);
        setting.mode = self.current()?.mode;
        self.write(setting)
    }

    /// Switches between the three controls and the eight bells.
    pub fn set_mode(&self, mode: EqMode) -> Result<()> {
        let mut setting = self.current()?;
        setting.mode = mode;
        self.write(setting)
    }

    /// Moves the three tone controls.
    pub fn set_simple(&self, simple: SimpleEq) -> Result<()> {
        let mut setting = self.current()?;
        setting.simple = simple;
        setting.mode = EqMode::Simple;
        self.write(setting)
    }

    /// Moves one bell: where it sits, how wide it is, how far it lifts.
    pub fn set_band(&self, index: usize, band: EqBand) -> Result<()> {
        let mut setting = self.current()?;
        if index >= ADVANCED_BAND_COUNT {
            return Err(CoreError::not_found("eq band", index));
        }

        if setting.advanced.len() != ADVANCED_BAND_COUNT {
            setting.advanced = default_advanced_bands();
        }
        setting.advanced[index] = band;
        setting.mode = EqMode::Advanced;
        self.write(setting)
    }

    /// Applies a setting to the filters without writing it down.
    ///
    /// What a control being dragged does. A parametric setting is twenty-eight
    /// rows, and a hand on a curve produces movement faster than any database
    /// wants to hear about it — so the sound follows at once and the writing
    /// waits for [`Self::commit`], which is what letting go is for.
    pub fn preview(&self, setting: EqSetting) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;

        self.ports.engine.set_eq(&setting)?;
        *self.current.write().unwrap_or_else(|err| err.into_inner()) = Some((profile_id, setting));
        Ok(())
    }

    /// Writes down what is set now.
    pub fn commit(&self) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let setting = self.current()?;

        self.store(profile_id, &setting)?;
        self.announce();
        Ok(())
    }

    /// Puts everything back to doing nothing.
    ///
    /// And forgets what the playing track was chosen to sound like, because
    /// that is what the press means: a listener who resets while a record is on
    /// is saying they no longer want that record treated specially, and leaving
    /// the choice in the table would bring it back the moment the track did.
    pub fn reset(&self) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        if let Some(track) = *self.playing.read().unwrap_or_else(|err| err.into_inner()) {
            self.ports.choices.forget(profile_id, track)?;
        }

        self.write(EqSetting::flat())
    }

    /// Saves what is set now under a name of the listener's own.
    ///
    /// Both halves of it, whichever mode was on screen: a saved sound behaves
    /// like a built-in one, moving the three controls in the simple mode and
    /// the eight bands in the advanced.
    pub fn save_as(&self, name: &str) -> Result<EqPreset> {
        let profile_id = self.context.require_active_profile()?;
        let name = self.usable_name(profile_id, name, None)?;

        let setting = self.current()?;
        let now = self.context.now();
        let preset = EqPreset {
            id: EqPresetId::new(),
            profile_id: Some(profile_id),
            name,
            is_builtin: false,
            mode: setting.mode,
            simple: setting.simple,
            advanced: setting.advanced,
            created_at: now,
            updated_at: now,
        };

        self.ports.presets.save(&preset)?;
        self.announce();
        Ok(preset)
    }

    /// Gives one of the listener's own presets another name.
    pub fn rename(&self, id: EqPresetId, name: &str) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let mut preset = self.own_preset(profile_id, id)?;

        preset.name = self.usable_name(profile_id, name, Some(id))?;
        preset.updated_at = self.context.now();

        self.ports.presets.save(&preset)?;
        self.announce();
        Ok(())
    }

    /// A name that is not empty and not already somebody else's.
    ///
    /// Checked here rather than left to the unique index, which can only fail —
    /// and fails in the vocabulary of a database. Compared without regard for
    /// case, because two sounds called "Late night" and "late night" are two
    /// ways of losing track of one.
    fn usable_name(
        &self,
        profile_id: ProfileId,
        name: &str,
        except: Option<EqPresetId>,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::invalid("eq preset", "a sound needs a name"));
        }

        let taken = self
            .ports
            .presets
            .list_for_profile(profile_id)?
            .into_iter()
            .any(|other| Some(other.id) != except && other.name.eq_ignore_ascii_case(name));

        if taken {
            return Err(CoreError::invalid(
                "eq preset",
                format!(
                    "there is already a sound called \"{name}\" —                      give this one another name, or rename that one from its ··· menu"
                ),
            ));
        }

        Ok(name.to_owned())
    }

    /// One of the listener's own presets, refusing the built-ins and everybody
    /// else's.
    fn own_preset(&self, profile_id: ProfileId, id: EqPresetId) -> Result<EqPreset> {
        let preset = self
            .ports
            .presets
            .get(id)?
            .ok_or_else(|| CoreError::not_found("eq preset", id))?;

        if preset.is_builtin || preset.profile_id != Some(profile_id) {
            return Err(CoreError::invalid(
                "eq preset",
                "that sound is not one of yours to change",
            ));
        }
        Ok(preset)
    }

    /// Forgets one of the listener's own presets. What is playing is unchanged.
    pub fn delete(&self, id: EqPresetId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.own_preset(profile_id, id)?;

        self.ports.presets.delete(id)?;
        self.announce();
        Ok(())
    }

    /// Applies a setting, stores it, and remembers it.
    ///
    /// In that order on purpose: the sound moves first because that is what was
    /// asked for, and a database that will not write is not a reason to keep
    /// playing what the listener has turned off.
    fn write(&self, setting: EqSetting) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;

        self.ports.engine.set_eq(&setting)?;
        self.store(profile_id, &setting)?;
        *self.current.write().unwrap_or_else(|err| err.into_inner()) = Some((profile_id, setting));

        self.announce();
        Ok(())
    }

    /// Reads the stored setting, falling back to flat where nothing was chosen.
    fn read_setting(&self, profile_id: ProfileId) -> Result<EqSetting> {
        let store = &self.context.settings;
        let mut setting = EqSetting::flat();

        if let Some(value) = store.profile_get(profile_id, MODE_KEY)? {
            setting.mode = EqMode::parse(value.as_text()?)?;
        }

        let mut simple = [GainDb::ZERO; 3];
        for (slot, key) in simple.iter_mut().zip(SIMPLE_KEYS) {
            if let Some(value) = store.profile_get(profile_id, key)? {
                *slot = GainDb::clamped(value.as_float()? as f32);
            }
        }
        setting.simple = SimpleEq {
            bass: simple[0],
            mid: simple[1],
            treble: simple[2],
        };

        for (index, band) in setting.advanced.iter_mut().enumerate() {
            let stored = (
                store.profile_get(profile_id, &band_key(index, "hz"))?,
                store.profile_get(profile_id, &band_key(index, "q"))?,
                store.profile_get(profile_id, &band_key(index, "db"))?,
            );
            let (Some(hz), Some(q), Some(db)) = stored else {
                continue;
            };

            // A band that will not rebuild is one somebody edited by hand into
            // something a filter cannot use. The default placement is a better
            // answer than refusing to start.
            let frequency_hz = u32::try_from(hz.as_integer()?).unwrap_or_default();
            if let Ok(rebuilt) = EqBand::new(
                frequency_hz,
                q.as_float()? as f32,
                GainDb::clamped(db.as_float()? as f32),
            ) {
                *band = rebuilt;
            }
        }

        Ok(setting)
    }

    /// Writes the setting out as flat scalars.
    ///
    /// Twenty-eight keys rather than one blob: `SettingValue` has four scalar
    /// shapes and deliberately no nesting, and this is the reason it does — a
    /// stored blob cannot be constrained, and these numbers reach a realtime
    /// filter.
    fn store(&self, profile_id: ProfileId, setting: &EqSetting) -> Result<()> {
        let store = &self.context.settings;
        let now = self.context.now();

        store.profile_set(
            profile_id,
            MODE_KEY,
            &SettingValue::Text(setting.mode.as_str().to_owned()),
            now,
        )?;

        let simple = [
            setting.simple.bass,
            setting.simple.mid,
            setting.simple.treble,
        ];
        for (gain, key) in simple.iter().zip(SIMPLE_KEYS) {
            store.profile_set(
                profile_id,
                key,
                &SettingValue::Float(f64::from(gain.as_db())),
                now,
            )?;
        }

        for (index, band) in setting.advanced.iter().enumerate() {
            store.profile_set(
                profile_id,
                &band_key(index, "hz"),
                &SettingValue::Integer(i64::from(band.frequency_hz())),
                now,
            )?;
            store.profile_set(
                profile_id,
                &band_key(index, "q"),
                &SettingValue::Float(f64::from(band.q())),
                now,
            )?;
            store.profile_set(
                profile_id,
                &band_key(index, "db"),
                &SettingValue::Float(f64::from(band.gain().as_db())),
                now,
            )?;
        }

        Ok(())
    }

    fn announce(&self) {
        self.context.events.publish(DomainEvent::EqChanged);
    }
}

/// Where one number of one band is kept.
fn band_key(index: usize, field: &str) -> String {
    format!("eq.band{index}.{field}")
}
