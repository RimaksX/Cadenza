//! The radio.

use super::*;

impl Controller {
    /// Re-reads the moods and what the station is doing.
    pub fn refresh_radio(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let radio_ui = window.global::<Radio>();

        let moods = match self.services.radio.moods() {
            Ok(moods) => moods,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        let rows: Vec<MoodRowData> = moods
            .iter()
            .map(|mood| MoodRowData {
                id: mood.id.to_string().into(),
                name: crate::text::tr_owned(mood.name.as_str()).into(),
                asks: radio_vm::asks_for(&mood.rules).into(),
            })
            .collect();
        radio_ui.set_moods(ModelRc::new(VecModel::from(rows)));

        let playing = self.services.radio.mood();
        radio_ui.set_mood(
            playing
                .as_ref()
                .map_or_else(String::new, |mood| crate::text::tr_owned(&mood.name))
                .into(),
        );
        radio_ui.set_summary(
            radio_vm::summary_line(playing.as_ref(), self.services.queue.view().pending).into(),
        );
    }

    /// Starts a station in the chosen mood, seeded by what is playing.
    pub fn start_radio(&self, id: &str) {
        self.run(|| {
            let mood_id = MoodId::parse(id)?;
            let seed = self
                .services
                .playback
                .view()
                .track
                .map(|track| track.media_file_id);

            let session = self.services.radio.start(mood_id, seed)?;
            let batch = self.services.radio.next_batch(MIN_BATCH_SIZE)?;
            self.services.queue.play_radio(session.id, &batch)
        });

        self.refresh_radio();
        self.refresh_queue();
        self.refresh_player();
    }

    /// Tells the station what the listener thinks of what is playing.
    pub fn judge_radio(&self, like: bool) {
        let Some(track) = self.services.playback.view().track else {
            return;
        };

        let verdict = if like {
            RadioFeedback::Like
        } else {
            RadioFeedback::Dislike
        };
        self.run(|| self.services.radio.feedback(track.media_file_id, verdict));
    }
}
