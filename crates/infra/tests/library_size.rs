//! What a full library costs, in milliseconds.
//!
//! Cadenza is sized for 1000–5000 tracks, and that asks for a
//! performance pass. This is the measurement that pass is made from: a real
//! database with five thousand tracks in it, timed on the path the window
//! actually walks: the whole-library read behind every refresh. It is what
//! showed that the same read was being paid for on every keystroke, which it
//! no longer is.
//!
//! Ignored by default. It is a measurement rather than an assertion: numbers
//! that mean something on the machine they were taken on and nothing at all on
//! a build agent sharing a disk with six other jobs.
//!
//! ```text
//! cargo test -p cadenza-infra --test library_size --release -- --ignored --nocapture
//! ```
//!
//! `--release`, because a debug build measures the compiler.

use std::path::PathBuf;
use std::time::Instant;

use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::ports::repositories::{
    MediaFileRepositoryPort, ProfileRepositoryPort, TrackRepositoryPort,
};
use cadenza_core::domain::profile::{Profile, ProfileName};
use cadenza_core::domain::track::Track;
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteMediaFileRepository, SqliteProfileRepository, SqliteTrackRepository,
};
use cadenza_testkit::TempDb;

/// The top of the size the player is built for.
const TRACKS: usize = 5_000;

fn took(label: &str, at: Instant) {
    println!(
        "{label:<46} {:>8.1} ms",
        at.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn what_a_full_library_costs() {
    let db = TempDb::new();
    let files = SqliteMediaFileRepository::new(db.pool().clone());
    let tracks = SqliteTrackRepository::new(db.pool().clone());

    let profile = Profile::new(
        ProfileName::new("Sasha").expect("a name"),
        Timestamp::from_millis(0),
    );
    let profile_id = profile.id;
    SqliteProfileRepository::new(db.pool().clone())
        .save(&profile)
        .expect("a profile");

    let filling = Instant::now();
    for index in 0..TRACKS {
        let file = MediaFile {
            id: MediaFileId::new(),
            path: PathBuf::from(format!(
                "C:/music/album {}/{index:05} track.flac",
                index / 12
            )),
            file_hash: None,
            file_size: 30_000_000,
            file_mtime: Timestamp::from_millis(0),
            format: AudioFormat::Flac,
            properties: AudioProperties {
                duration: DurationMs::from_secs(200),
                sample_rate: 44_100,
                channels: 2,
                bitrate: None,
            },
            metadata_version: None,
            metadata_extracted_at: None,
            state: FileState::Available,
            created_at: Timestamp::from_millis(0),
            updated_at: Timestamp::from_millis(0),
        };
        files.save(&file).expect("catalogued");

        tracks
            .save(&Track {
                profile_id,
                media_file_id: file.id,
                // Spread across the alphabet, so the ordering has work to do
                // and a search has more to reject than to accept.
                title: format!(
                    "{} song about {index}",
                    char::from(b'a' + u8::try_from(index % 26).expect("under 26"))
                ),
                artist_id: None,
                album_id: None,
                track_no: None,
                disc_no: None,
                year: None,
                added_at: Timestamp::from_millis(0),
                removed_at: None,
            })
            .expect("in the library");
    }
    took(&format!("filling {TRACKS} tracks"), filling);

    // What every refresh of the library does.
    for round in 0..3 {
        let at = Instant::now();
        let summaries = tracks
            .summaries_for_profile(profile_id)
            .expect("the library");
        assert_eq!(summaries.len(), TRACKS);
        took(&format!("summaries_for_profile (round {round})"), at);
    }
}
