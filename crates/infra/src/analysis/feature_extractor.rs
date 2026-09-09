//! One file in, one row of features out.

use std::path::Path;

use cadenza_core::Result;
use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::ports::clock::ClockPort;
use cadenza_core::domain::ports::feature_extractor::FeatureExtractorPort;
use cadenza_core::domain::track::TrackFeatures;
use cadenza_core::domain::value_objects::Bpm;
use std::sync::Arc;

use super::dsp::Spectra;
use super::{activity, bpm, danceability, decode, energy, key, spectral, valence};

/// Which algorithm produced a row of features.
///
/// Stored beside every answer and compared before any file is analysed again.
/// **Change this whenever the numbers would come out differently** — a new
/// window length, a different profile, a fixed bug — because a library holding
/// two versions of a feature is a library where similarity compares one track's
/// measurement with another track's mistake.
pub const VERSION: &str = "dsp-2";

/// Local DSP feature extraction.
pub struct DspFeatureExtractor {
    clock: Arc<dyn ClockPort>,
}

impl DspFeatureExtractor {
    /// Binds the extractor to a clock, which stamps every answer.
    #[must_use]
    pub fn new(clock: Arc<dyn ClockPort>) -> Self {
        Self { clock }
    }
}

impl FeatureExtractorPort for DspFeatureExtractor {
    fn version(&self) -> &str {
        VERSION
    }

    fn extract(&self, media_file_id: MediaFileId, path: &Path) -> Result<TrackFeatures> {
        let window = decode::read_window(path)?;
        let spectra = Spectra::of(&window);

        // Order matters only in that later measurements reuse earlier ones:
        // energy asks brightness what the top end is doing, and both
        // danceability and valence are read off the tempo rather than
        // recomputed from the signal.
        let brightness = spectral::measure(&spectra);
        let busy = activity::measure(&spectra);
        let level = energy::measure(&window, &brightness, &busy);
        let tempo = bpm::measure(&spectra);
        let estimate = key::measure(&spectra);

        // A tempo outside what the domain accepts is not an answer worth
        // storing: better an empty column, which every caller already handles,
        // than a number that would be trusted.
        let tempo_bpm = tempo.bpm.and_then(|value| Bpm::new(value).ok());

        Ok(TrackFeatures {
            media_file_id,
            bpm: tempo_bpm,
            bpm_confidence: if tempo_bpm.is_some() {
                tempo.confidence
            } else {
                0.0
            },
            key: estimate.key,
            energy: level.energy,
            loudness: level.loudness,
            spectral_centroid: brightness.centroid,
            spectral_rolloff: brightness.rolloff,
            danceability: danceability::measure(&tempo),
            valence: valence::measure(estimate.key, &tempo, &brightness),
            tempo_stability: tempo.stability,
            dynamic_range: level.dynamic_range,
            extractor_version: VERSION.to_owned(),
            analyzed_at: self.clock.now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DspFeatureExtractor, VERSION};
    use cadenza_core::domain::ids::MediaFileId;
    use cadenza_core::domain::ports::feature_extractor::FeatureExtractorPort;
    use cadenza_testkit::TempDir;
    use cadenza_testkit::TestClock;
    use cadenza_testkit::audio_fixtures::write_wav;
    use std::sync::Arc;

    #[test]
    fn a_real_file_comes_back_with_every_column_filled_and_in_range() {
        let directory = TempDir::new("extractor");
        let path = write_wav(directory.path(), "tone.wav", 3, 9_000);

        let extractor = DspFeatureExtractor::new(Arc::new(TestClock::default()));
        let id = MediaFileId::new();
        let features = extractor.extract(id, &path).expect("analysed");

        assert_eq!(features.media_file_id, id);
        assert_eq!(features.extractor_version, VERSION);

        for (name, value) in [
            ("energy", features.energy),
            ("loudness", features.loudness),
            ("spectral_centroid", features.spectral_centroid),
            ("spectral_rolloff", features.spectral_rolloff),
            ("danceability", features.danceability),
            ("valence", features.valence),
            ("tempo_stability", features.tempo_stability),
            ("dynamic_range", features.dynamic_range),
            ("bpm_confidence", features.bpm_confidence),
        ] {
            assert!(
                (0.0..=1.0).contains(&value),
                "{name} came out as {value}, which the table would refuse"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_audio_is_an_error_rather_than_a_guess() {
        let directory = TempDir::new("extractor-bad");
        let path = directory.path().join("not-audio.wav");
        std::fs::write(&path, b"this is not a wave file").expect("written");

        let extractor = DspFeatureExtractor::new(Arc::new(TestClock::default()));
        assert!(extractor.extract(MediaFileId::new(), &path).is_err());
    }
}
