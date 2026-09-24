//! What is playing: the transport, the bar, the cover and the line, and the heart.

use super::*;

impl Controller {
    /// Re-reads the transport. Called on every command and on the tick.
    pub fn refresh_player(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let shown = player_vm::fields(&self.services.playback.view());

        let player = window.global::<Player>();
        player.set_title(shown.title.as_str().into());
        player.set_subtitle(shown.subtitle.as_str().into());
        player.set_initial(shown.initial.as_str().into());
        player.set_position(shown.position.as_str().into());
        player.set_duration(shown.duration.as_str().into());
        player.set_playing_id(shown.playing_id.as_str().into());

        // And where that row is in the list as it is currently ordered and
        // filtered, so that the player bar can point at it. Counted here
        // because the markup cannot search a model, and recounted whenever the
        // player changes: a track that finishes moves the answer.
        window
            .global::<Library>()
            .set_playing_row(self.playing_row(&shown.playing_id));
        player.set_favourite(self.is_favourite(&shown.playing_id));
        player.set_progress(shown.progress);
        player.set_playing(shown.playing);
        player.set_loaded(shown.loaded);
        player.set_muted(shown.muted);
        player.set_volume(shown.volume);
        player.set_sleep(shown.sleep.as_str().into());

        let speed = self
            .services
            .playback
            .settings()
            .map(|settings| settings.speed)
            .unwrap_or_default();
        player.set_speed_percent(i32::from(speed.percent()));

        self.refresh_cover(false);
        self.refresh_waveform(&window, &shown.playing_id);

        let queue = self.services.queue.view();
        player.set_shuffle(queue.shuffle);
        player.set_repeating(queue.repeats());
        player.set_repeat_one(queue.repeat == RepeatMode::One);
        player.set_has_next(queue.has_next);
        player.set_has_previous(queue.has_previous);
    }

    /// Advances when the current track has run out.
    ///
    /// Called from the tick alongside [`Self::refresh_player`], because nothing
    /// else can: the audio callback is forbidden from calling into the
    /// application layer.
    pub fn poll_queue(&self) {
        match self.services.queue.poll() {
            // Only when a track actually changed, so the tick does not turn a
            // listing query into a background load.
            Ok(true) => {
                self.refresh_queue();
                // The track that ended is now the one heard last.
                self.refresh_home();
                // A track that ended is a listen that was just written down,
                // which is the only thing that can move the favourites list.
                // It writes nothing when the order has not changed, so most of
                // these cost two reads.
                self.refresh_favourites();
            }
            Ok(false) => {}
            Err(err) => self.report(&err),
        }
    }

    /// Whether what is playing is in the favourites.
    ///
    /// Kept, and forgotten wherever the list could have changed. It changes
    /// from four places - the heart, the row menu, the playlist page, and the
    /// count rebuilding itself when a track ends. The first three are commands,
    /// and every command goes through [`Self::run`], which forgets it; the
    /// last is [`Self::refresh_favourites`], which does too.
    ///
    /// A failure here is a heart drawn empty, which is what it would be drawn
    /// as anyway before anything is playing.
    pub(super) fn is_favourite(&self, playing_id: &str) -> bool {
        if let Some((about, answer)) = &*self.favourite.borrow()
            && about == playing_id
        {
            return *answer;
        }
        let answer = MediaFileId::parse(playing_id)
            .and_then(|media_file_id| self.services.playlists.is_favourite(media_file_id))
            .unwrap_or(false);
        *self.favourite.borrow_mut() = Some((playing_id.to_owned(), answer));
        answer
    }

    /// Puts what is playing in the favourites, or takes it out.
    pub fn toggle_favourite(&self) {
        let Some(media_file_id) = self
            .services
            .playback
            .view()
            .track
            .map(|track| track.media_file_id)
        else {
            return;
        };

        let wanted = !self
            .services
            .playlists
            .is_favourite(media_file_id)
            .unwrap_or(false);

        self.run(|| self.services.playlists.set_favourite(media_file_id, wanted));
        self.refresh_player();
        self.refresh_playlists();
    }

    /// Starts the track a row identifies, with the rest of the library behind it.
    pub fn play(&self, id: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services.queue.play_from_library(media_file_id)
        });
        self.refresh_queue();
        // Choosing a track ends the station, so the radio screen is now saying
        // something that is no longer true — that a mood is playing, and that
        // there is something to give a verdict about.
        self.refresh_radio();
    }

    /// Moves to the next track.
    pub fn next(&self) {
        // Pressing next during a station is a verdict on what is playing — the
        // weakest kind, but one radio has to learn from. Recorded before the
        // track changes, because after it there is nothing to point at. A track
        // that ran out on its own is not a skip and does not come through here.
        if self.services.radio.session().is_some()
            && let Some(track) = self.services.playback.view().track
        {
            self.run(|| {
                self.services
                    .radio
                    .feedback(track.media_file_id, RadioFeedback::Skip)
            });
        }

        self.run(|| self.services.queue.next());
        self.refresh_queue();
        self.refresh_radio();
    }

    /// Restarts the track, or moves back to the previous one.
    pub fn previous(&self) {
        self.run(|| self.services.queue.previous());
        self.refresh_queue();
    }

    /// Turns shuffle on or off.
    pub fn toggle_shuffle(&self) {
        self.run(|| self.services.queue.toggle_shuffle());
        self.refresh_queue();
    }

    /// Steps through the repeat modes.
    pub fn cycle_repeat(&self) {
        self.run(|| self.services.queue.cycle_repeat());
        self.refresh_queue();
    }

    /// Pauses or resumes.
    pub fn toggle_play(&self) {
        self.run(|| self.services.playback.toggle());
    }

    /// Jumps to a fraction of the track.
    ///
    /// The bar reports where it was clicked, not a time: it knows its own width
    /// and nothing about how long the track is.
    pub fn seek(&self, fraction: f32) {
        self.run(|| {
            let view = self.services.playback.view();
            let millis =
                (f64::from(fraction.clamp(0.0, 1.0)) * view.duration.as_millis() as f64) as u64;
            self.services
                .playback
                .seek(PlaybackPosition::from_millis(millis))
        });
    }

    /// Sets the output level.
    pub fn set_volume(&self, level: f32) {
        self.run(|| self.services.playback.set_volume(Volume::clamped(level)));
    }

    /// Puts the playing track's cover in the player bar.
    ///
    /// Only when the track changed, or when somebody has just chosen one: this
    /// is asked four times a second, and reading an image off the disk that
    /// often would be paid for in frames for a picture that changes when the
    /// music does.
    pub(super) fn refresh_cover(&self, force: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let player = window.global::<Player>();
        let playing = player.get_playing_id().to_string();
        if !force && *self.showing_cover.borrow() == playing {
            return;
        }
        *self.showing_cover.borrow_mut() = playing.clone();

        let cover = MediaFileId::parse(&playing)
            .ok()
            .and_then(|id| self.services.library.cover_for(id).ok().flatten())
            .and_then(|path| slint::Image::load_from_path(&path).ok())
            .unwrap_or_default();

        // The cover's colour for the panel, sampled from the picture already
        // decoded rather than read again.
        let tint = tint_of(&cover);
        player.set_tinted(tint.is_some());
        if let Some(tint) = tint {
            player.set_tint(tint);
        }
        player.set_cover(cover);
    }

    /// The shape of what is playing, for the progress line: kept, or asked
    /// for the first time a track is heard and picked up when it is ready.
    pub(super) fn refresh_waveform(&self, window: &AppWindow, playing: &str) {
        let changed = self.showing_wave.borrow().0 != playing;
        if !changed && self.showing_wave.borrow().1 {
            return;
        }
        let player = window.global::<Player>();
        if changed {
            *self.showing_wave.borrow_mut() = (playing.to_owned(), false);
            player.set_waveform(ModelRc::default());
        }
        let Ok(id) = MediaFileId::parse(playing) else {
            return;
        };
        match self.services.waveforms.get(id) {
            Ok(Some(levels)) => {
                let shape = cadenza_core::domain::waveform::heights(&levels);
                player.set_waveform(ModelRc::new(VecModel::from(shape)));
                self.showing_wave.borrow_mut().1 = true;
            }
            Ok(None) if changed => self.services.waveforms.request(id),
            Ok(None) => {}
            // A plain line, and no more asking: the waveform is decoration.
            Err(_) => self.showing_wave.borrow_mut().1 = true,
        }
    }

    /// Silences output, or restores it.
    pub fn toggle_mute(&self) {
        self.run(|| self.services.playback.toggle_mute());
    }

    /// Sets the sleep timer from its menu: minutes, "end" or "off".
    pub fn set_sleep(&self, choice: &str) {
        let playback = &self.services.playback;
        match choice {
            "end" => self.run(|| playback.set_sleep(Some(SleepTimer::EndOfTrack))),
            "off" => self.run(|| playback.set_sleep(None)),
            minutes => {
                if let Ok(minutes) = minutes.parse::<u64>() {
                    self.run(|| playback.sleep_after(DurationMs::from_secs(minutes * 60)));
                }
            }
        }
    }

    /// Runs the sleep timer on: lowers the level as it runs out, pauses at the
    /// end.
    pub fn tick_sleep(&self) {
        if let Err(err) = self.services.playback.tick_sleep() {
            self.report(&err);
        }
    }
}
