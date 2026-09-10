//! Background analysis over a real database.
//!
//! The three things analysis is judged on: the answers are saved, a file already
//! analysed is not analysed again, and a file that cannot be analysed does not
//! come back for ever.

use std::path::PathBuf;
use std::sync::Arc;

use cadenza_core::application::AppContext;
use cadenza_core::application::services::{AnalysisPorts, AnalysisService};
use cadenza_core::domain::analysis::MAX_ATTEMPTS;
use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::ports::repositories::MediaFileRepositoryPort;
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_infra::analysis::{DspFeatureExtractor, VERSION};
use cadenza_infra::db::repositories::{
    SqliteAnalysisJobRepository, SqliteMediaFileRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackFeaturesRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::audio_fixtures::write_wav;
use cadenza_testkit::{TempDb, TempDir, TestClock};

struct Harness {
    analysis: AnalysisService,
    media_files: SqliteMediaFileRepository,
    /// Held so the files and the database outlive the test.
    _db: TempDb,
    directory: TempDir,
}

fn harness() -> Harness {
    let db = TempDb::new();

    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));

    let analysis = AnalysisService::new(
        context,
        AnalysisPorts {
            jobs: Arc::new(SqliteAnalysisJobRepository::new(db.pool().clone())),
            features: Arc::new(SqliteTrackFeaturesRepository::new(db.pool().clone())),
            media_files: Arc::new(SqliteMediaFileRepository::new(db.pool().clone())),
            extractor: Arc::new(DspFeatureExtractor::new(Arc::new(TestClock::default()))),
        },
    );

    Harness {
        media_files: SqliteMediaFileRepository::new(db.pool().clone()),
        analysis,
        _db: db,
        directory: TempDir::new("analysis-files"),
    }
}

impl Harness {
    /// Puts a real, playable file in the catalogue.
    fn catalogue(&self, name: &str) -> MediaFileId {
        let path = write_wav(self.directory.path(), name, 3, 10_000);
        self.record(path, FileState::Available)
    }

    /// Puts something in the catalogue that is not audio at all.
    fn catalogue_rubbish(&self, name: &str) -> MediaFileId {
        let path = self.directory.path().join(name);
        std::fs::write(&path, b"not a wave file").expect("written");
        self.record(path, FileState::Available)
    }

    fn record(&self, path: PathBuf, state: FileState) -> MediaFileId {
        let media_file = MediaFile {
            id: MediaFileId::new(),
            path,
            file_hash: None,
            file_size: 1_024,
            file_mtime: Timestamp::from_millis(0),
            format: AudioFormat::Wav,
            properties: AudioProperties {
                duration: DurationMs::from_secs(3),
                sample_rate: 44_100,
                channels: 2,
                bitrate: None,
            },
            metadata_version: None,
            metadata_extracted_at: None,
            state,
            created_at: Timestamp::from_millis(0),
            updated_at: Timestamp::from_millis(0),
        };
        self.media_files.save(&media_file).expect("catalogued");
        media_file.id
    }

    /// Runs every queued job to its end.
    fn drain(&self) -> usize {
        let mut done = 0;
        while self.analysis.run_next().expect("analysed").is_some() {
            done += 1;
        }
        done
    }
}

#[test]
fn a_catalogued_file_is_queued_analysed_and_remembered() {
    let harness = harness();
    let id = harness.catalogue("one.wav");

    assert_eq!(harness.analysis.top_up().expect("queued"), 1);
    assert_eq!(harness.analysis.progress().expect("progress").pending, 1);

    assert_eq!(harness.drain(), 1);

    let features = harness
        .analysis
        .features(id)
        .expect("read")
        .expect("analysed");
    assert_eq!(features.media_file_id, id);
    assert_eq!(features.extractor_version, VERSION);

    let progress = harness.analysis.progress().expect("progress");
    assert_eq!(progress.analysed, 1);
    assert!(progress.is_settled(), "and nothing is left waiting");
}

#[test]
fn a_file_already_analysed_is_never_analysed_again() {
    let harness = harness();
    harness.catalogue("one.wav");

    assert_eq!(harness.analysis.top_up().expect("queued"), 1);
    harness.drain();

    // The whole of "повторный анализ не происходит": asking again finds
    // nothing to do, however often it is asked.
    for _ in 0..3 {
        assert_eq!(
            harness.analysis.top_up().expect("queued"),
            0,
            "the answer is already known"
        );
    }
    assert_eq!(harness.drain(), 0);
}

#[test]
fn queueing_the_same_file_twice_queues_it_once() {
    let harness = harness();
    harness.catalogue("one.wav");

    assert_eq!(harness.analysis.top_up().expect("queued"), 1);
    assert_eq!(
        harness.analysis.top_up().expect("queued"),
        0,
        "it is already waiting"
    );
    assert_eq!(harness.analysis.progress().expect("progress").pending, 1);
}

#[test]
fn a_file_that_cannot_be_analysed_is_given_up_on_rather_than_retried_for_ever() {
    let harness = harness();
    harness.catalogue_rubbish("broken.wav");

    // Each attempt puts it back until the attempts run out. The loop is bounded
    // by the test, not by the code, which is the point: without the attempt
    // count this would never end.
    let mut attempts = 0;
    while harness.analysis.top_up().expect("queued") > 0 || harness.drain() > 0 {
        attempts += 1;
        assert!(attempts < 10, "a broken file was retried {attempts} times");
    }

    let progress = harness.analysis.progress().expect("progress");
    assert_eq!(progress.analysed, 0, "nothing was learned about it");
    assert!(progress.is_settled(), "and nothing is still waiting");
}

#[test]
fn a_missing_file_fails_its_job_without_stopping_the_others() {
    let harness = harness();
    let good = harness.catalogue("good.wav");
    harness.catalogue_rubbish("bad.wav");

    assert_eq!(harness.analysis.top_up().expect("queued"), 2);

    // One good file analysed once, and one bad file attempted its three times:
    // a failure goes back to the queue until the attempts run out, which is why
    // draining takes more passes than there are files.
    assert_eq!(harness.drain(), 1 + usize::from(MAX_ATTEMPTS));

    assert!(
        harness.analysis.features(good).expect("read").is_some(),
        "the good one still got its answer"
    );
    assert_eq!(harness.analysis.progress().expect("progress").analysed, 1);
}

/// What analysing real music actually costs.
///
/// Ignored: it needs a folder of real files, which a build server has not got.
/// Run it by hand, against your own music:
///
/// ```text
/// cargo test --release -p cadenza-infra --test analysis -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a folder of real music"]
fn what_analysing_a_library_costs() {
    use cadenza_core::domain::ports::feature_extractor::FeatureExtractorPort;
    use std::time::Instant;

    let folder = std::path::Path::new("C:/Music");
    let Ok(entries) = std::fs::read_dir(folder) else {
        eprintln!("{} is not there — nothing to measure", folder.display());
        return;
    };

    let extractor = DspFeatureExtractor::new(Arc::new(TestClock::default()));
    let mut total = std::time::Duration::ZERO;
    let mut files = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let at = Instant::now();
        let Ok(features) = extractor.extract(MediaFileId::new(), &path) else {
            continue;
        };
        let took = at.elapsed();

        total += took;
        files += 1;
        println!(
            "{:>6.0}ms  {:>7}  conf {:.2}  energy {:.2}  {}",
            took.as_secs_f32() * 1_000.0,
            features
                .bpm
                .map_or_else(|| "-".to_owned(), |bpm| format!("{:.0} bpm", bpm.as_f32())),
            features.bpm_confidence,
            features.energy,
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    assert!(
        files > 0,
        "no file in {} could be analysed",
        folder.display()
    );

    let each = total.as_secs_f32() / files as f32;
    println!(
        "\n{files} file(s), {:.0}ms each — a five-thousand-track library is about \
         {:.0} minutes of one core, or {:.1} hours at the background share",
        each * 1_000.0,
        each * 5_000.0 / 60.0,
        each * 5_000.0 / 0.2 / 3_600.0
    );
}
