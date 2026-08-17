//! Integration tests for the database: migrations, pragmas, and the constraints
//! the schema is supposed to enforce.
//!
//! These run against a real file with the real migration runner, because the
//! things worth testing here — WAL, foreign keys, cascades, partial indexes —
//! are exactly the things an in-memory shortcut would not exercise.
//!
//! Rows are inserted with plain SQL rather than through repositories. There are
//! no repositories yet, and a schema test that goes through a mapping layer
//! cannot tell a schema bug from a mapping bug.

use cadenza_infra::db::migrations::{self, LATEST_VERSION};
use cadenza_infra::db::sqlite;
use cadenza_testkit::TempDb;
use rusqlite::{Connection, Error, ErrorCode};

/// A fixed instant, so nothing here depends on the wall clock.
const NOW: i64 = 1_754_611_200_000;

/// Every table PROJECT_MASTER section 7 defines, plus migration bookkeeping.
const EXPECTED_TABLES: [&str; 21] = [
    "schema_migrations",
    "profiles",
    "media_files",
    "track_features",
    "artists",
    "albums",
    "genres",
    "profile_tracks",
    "track_genres",
    "playlists",
    "playlist_items",
    "play_events",
    "daily_track_stats",
    "daily_artist_stats",
    "daily_genre_stats",
    "daily_radio_stats",
    "mood_presets",
    "radio_sessions",
    "radio_session_items",
    "eq_presets",
    "profile_folders",
];

fn table_exists(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .is_ok()
}

fn insert_profile(connection: &Connection, id: &str, name: &str) {
    connection
        .execute(
            "INSERT INTO profiles (id, name, created_at) VALUES (?1, ?2, ?3)",
            (id, name, NOW),
        )
        .unwrap_or_else(|err| panic!("inserting profile {name}: {err}"));
}

fn insert_media_file(connection: &Connection, id: &str, path: &str) {
    connection
        .execute(
            "INSERT INTO media_files
                 (id, path, file_size, file_mtime, format,
                  duration_ms, sample_rate, channels, created_at, updated_at)
             VALUES (?1, ?2, 41231884, ?3, 'flac', 215000, 44100, 2, ?3, ?3)",
            (id, path, NOW),
        )
        .unwrap_or_else(|err| panic!("inserting media file {path}: {err}"));
}

fn insert_profile_track(connection: &Connection, profile_id: &str, media_file_id: &str) {
    connection
        .execute(
            "INSERT INTO profile_tracks (profile_id, media_file_id, title, added_at)
             VALUES (?1, ?2, 'Untitled', ?3)",
            (profile_id, media_file_id, NOW),
        )
        .unwrap_or_else(|err| panic!("inserting profile track: {err}"));
}

fn count(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|err| panic!("counting with {sql}: {err}"))
}

fn is_constraint_violation(err: &Error) -> bool {
    matches!(err, Error::SqliteFailure(failure, _)
        if failure.code == ErrorCode::ConstraintViolation)
}

#[test]
fn migrations_bring_a_fresh_database_to_the_latest_version() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    let version = migrations::current_version(&connection).expect("a version");
    assert_eq!(version, Some(LATEST_VERSION));

    let applied = migrations::applied_versions(&connection).expect("applied versions");
    assert_eq!(
        applied,
        (1..=LATEST_VERSION).collect::<Vec<_>>(),
        "every migration should have run exactly once, in order"
    );
}

#[test]
fn applying_migrations_again_does_nothing() {
    let db = TempDb::new();
    let mut connection = db.pool().get().expect("a connection");

    let second_run = migrations::apply(&mut connection).expect("a second run");
    assert!(
        second_run.is_empty(),
        "an up-to-date database should need no work, got {second_run:?}"
    );
}

#[test]
fn every_table_the_schema_defines_exists() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    for table in EXPECTED_TABLES {
        assert!(table_exists(&connection, table), "{table} is missing");
    }

    assert!(
        table_exists(&connection, "app_settings") && table_exists(&connection, "profile_settings"),
        "settings storage is missing"
    );
    assert!(
        table_exists(&connection, "analysis_jobs") && table_exists(&connection, "import_review"),
        "analysis or review storage is missing"
    );
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let db = TempDb::new();
    let mut connection = db.pool().get().expect("a connection");

    connection
        .execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (999, 'future', ?1)",
            [NOW],
        )
        .expect("pretending a newer Cadenza has been here");

    let err = migrations::apply(&mut connection)
        .expect_err("a database from the future must not be touched");
    assert!(
        err.to_string().contains("newer version"),
        "the error should say why, got: {err}"
    );
}

#[test]
fn a_foreign_migration_table_is_refused_rather_than_half_applied() {
    // Found by running the binary on a real machine: a database left by an
    // earlier build had a schema_migrations table with only a version column.
    // Its rows were read as Cadenza's own, five migrations were skipped, and the
    // sixth failed with a message about a missing column — nowhere near the
    // actual problem.
    let directory = std::env::temp_dir().join(format!(
        "cadenza-foreign-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&directory).expect("a directory");
    let path = directory.join("app.db");

    {
        let mut connection = sqlite::open(&path).expect("a connection");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY) STRICT;
                 INSERT INTO schema_migrations (version)
                 VALUES (1), (2), (3), (4), (5);",
            )
            .expect("a table written by something else");

        let err = migrations::apply(&mut connection).expect_err("this is not our database");
        let message = err.to_string();
        assert!(
            message.contains("not written by this version"),
            "the error should point at the file, got: {message}"
        );

        assert!(
            !table_exists(&connection, "profiles"),
            "nothing may be created in a database we refused to migrate"
        );
    }

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn every_pooled_connection_enforces_foreign_keys_and_uses_wal() {
    let db = TempDb::new();

    // Hold them all at once: a pragma applied to only the first connection
    // handed out is the exact bug this guards against.
    let connections: Vec<_> = (0..db.pool().size())
        .map(|_| db.pool().get().expect("a connection"))
        .collect();

    for connection in &connections {
        assert_eq!(
            sqlite::pragma_i64(connection, "foreign_keys").expect("the pragma"),
            1,
            "foreign keys are per-connection and must be on for all of them"
        );

        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("the journal mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");
    }
}

#[test]
fn the_pool_takes_connections_back_when_they_are_dropped() {
    let db = TempDb::new();
    let size = db.pool().size();

    // Exhaust the pool, then release it. A guard that failed to return its
    // connection would make the next round block until the checkout timeout.
    for _ in 0..3 {
        let round: Vec<_> = (0..size)
            .map(|_| db.pool().get().expect("a connection"))
            .collect();
        assert_eq!(round.len(), size);
    }
}

#[test]
fn a_row_cannot_reference_a_profile_that_does_not_exist() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_media_file(&connection, "file-1", "D:/Music/a.flac");

    let err = connection
        .execute(
            "INSERT INTO profile_tracks (profile_id, media_file_id, title, added_at)
             VALUES ('nobody', 'file-1', 'Untitled', ?1)",
            [NOW],
        )
        .expect_err("an orphan track must be rejected");
    assert!(is_constraint_violation(&err), "got {err}");
}

#[test]
fn deleting_a_profile_takes_its_data_with_it() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_profile(&connection, "profile-1", "Sasha");
    insert_media_file(&connection, "file-1", "D:/Music/a.flac");
    insert_profile_track(&connection, "profile-1", "file-1");
    connection
        .execute(
            "INSERT INTO playlists (id, profile_id, name, created_at, updated_at)
             VALUES ('playlist-1', 'profile-1', 'Late night', ?1, ?1)",
            [NOW],
        )
        .expect("inserting a playlist");
    connection
        .execute(
            "INSERT INTO play_events
                 (id, profile_id, media_file_id, source, started_at, played_ms, duration_ms)
             VALUES ('event-1', 'profile-1', 'file-1', 'library', ?1, 120000, 215000)",
            [NOW],
        )
        .expect("inserting a play event");

    connection
        .execute("DELETE FROM profiles WHERE id = 'profile-1'", [])
        .expect("deleting the profile");

    assert_eq!(count(&connection, "SELECT COUNT(*) FROM profile_tracks"), 0);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM playlists"), 0);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM play_events"), 0);

    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM media_files"),
        1,
        "the file itself belongs to the machine, not the listener, and must survive"
    );
}

#[test]
fn one_profile_removing_a_track_leaves_the_other_untouched() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_profile(&connection, "profile-1", "Sasha");
    insert_profile(&connection, "profile-2", "Kim");
    insert_media_file(&connection, "file-1", "D:/Music/a.flac");
    insert_profile_track(&connection, "profile-1", "file-1");
    insert_profile_track(&connection, "profile-2", "file-1");

    connection
        .execute(
            "UPDATE profile_tracks SET removed_at = ?1
             WHERE profile_id = 'profile-1' AND media_file_id = 'file-1'",
            [NOW],
        )
        .expect("tombstoning one profile's copy");

    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM profile_tracks
             WHERE profile_id = 'profile-2' AND removed_at IS NULL"
        ),
        1,
        "removing a track from one library must not touch another profile's"
    );
}

#[test]
fn a_profile_cannot_keep_history_longer_than_the_policy_allows() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    let err = connection
        .execute(
            "INSERT INTO profiles (id, name, created_at, history_retention_days)
             VALUES ('profile-1', 'Sasha', ?1, 365)",
            [NOW],
        )
        .expect_err("365 days of history must be rejected");
    assert!(is_constraint_violation(&err), "got {err}");

    connection
        .execute(
            "INSERT INTO profiles (id, name, created_at, history_retention_days)
             VALUES ('profile-2', 'Kim', ?1, 7)",
            [NOW],
        )
        .expect("a shorter window is allowed");
}

#[test]
fn a_listen_cannot_be_both_completed_and_skipped() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_profile(&connection, "profile-1", "Sasha");
    insert_media_file(&connection, "file-1", "D:/Music/a.flac");

    let err = connection
        .execute(
            "INSERT INTO play_events
                 (id, profile_id, media_file_id, source, started_at,
                  played_ms, duration_ms, completed, skipped)
             VALUES ('event-1', 'profile-1', 'file-1', 'library', ?1, 120000, 215000, 1, 1)",
            [NOW],
        )
        .expect_err("the two flags contradict each other");
    assert!(is_constraint_violation(&err), "got {err}");
}

#[test]
fn only_a_radio_listen_may_name_a_radio_session() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_profile(&connection, "profile-1", "Sasha");
    insert_media_file(&connection, "file-1", "D:/Music/a.flac");

    let err = connection
        .execute(
            "INSERT INTO play_events
                 (id, profile_id, media_file_id, source, radio_session_id,
                  started_at, played_ms, duration_ms)
             VALUES ('event-1', 'profile-1', 'file-1', 'library', 'session-1',
                     ?1, 120000, 215000)",
            [NOW],
        )
        .expect_err("a library listen has no radio session");
    assert!(is_constraint_violation(&err), "got {err}");
}

#[test]
fn unknown_enum_values_are_rejected() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    let bad_theme = connection.execute(
        "INSERT INTO profiles (id, name, created_at, theme) VALUES ('p', 'Sasha', ?1, 'solarized')",
        [NOW],
    );
    assert!(bad_theme.is_err(), "an unknown theme must not be stored");

    let bad_format = connection.execute(
        "INSERT INTO media_files
             (id, path, file_size, file_mtime, format,
              duration_ms, sample_rate, channels, created_at, updated_at)
         VALUES ('f', 'D:/Music/a.ogg', 1, ?1, 'ogg', 1000, 44100, 2, ?1, ?1)",
        [NOW],
    );
    assert!(
        bad_format.is_err(),
        "Vorbis is not one of the supported formats"
    );
}

#[test]
fn a_builtin_preset_may_not_belong_to_a_profile() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_profile(&connection, "profile-1", "Sasha");

    let err = connection
        .execute(
            "INSERT INTO mood_presets (id, profile_id, name, is_builtin, created_at, updated_at)
             VALUES ('mood-1', 'profile-1', 'Sunday morning', 1, ?1, ?1)",
            [NOW],
        )
        .expect_err("a profile-owned preset claiming to be built-in must be rejected");
    assert!(is_constraint_violation(&err), "got {err}");

    connection
        .execute(
            "INSERT INTO mood_presets (id, profile_id, name, is_builtin, created_at, updated_at)
             -- Not one of the names migration 17 ships, which the unique
             -- index would otherwise catch before the CHECK under test.
             VALUES ('mood-2', NULL, 'Sunday morning', 1, ?1, ?1)",
            [NOW],
        )
        .expect("a genuine built-in is fine");
}

#[test]
fn an_equaliser_band_cannot_exceed_the_gain_range() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    let err = connection
        .execute(
            "INSERT INTO eq_presets (id, name, is_builtin, simple_bass_gain, created_at, updated_at)
             VALUES ('eq-1', 'Earthquake', 1, 30.0, ?1, ?1)",
            [NOW],
        )
        .expect_err("+30 dB is outside the range the domain allows");
    assert!(is_constraint_violation(&err), "got {err}");
}

#[test]
fn only_one_analysis_job_may_be_outstanding_per_file_and_kind() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_media_file(&connection, "file-1", "D:/Music/a.flac");

    connection
        .execute(
            "INSERT INTO analysis_jobs (id, media_file_id, kind, created_at, updated_at)
             VALUES ('job-1', 'file-1', 'features', ?1, ?1)",
            [NOW],
        )
        .expect("queuing work");

    let err = connection
        .execute(
            "INSERT INTO analysis_jobs (id, media_file_id, kind, created_at, updated_at)
             VALUES ('job-2', 'file-1', 'features', ?1, ?1)",
            [NOW],
        )
        .expect_err("the same work must not queue twice");
    assert!(is_constraint_violation(&err), "got {err}");

    // A different kind of work on the same file is not a duplicate.
    connection
        .execute(
            "INSERT INTO analysis_jobs (id, media_file_id, kind, created_at, updated_at)
             VALUES ('job-3', 'file-1', 'hash', ?1, ?1)",
            [NOW],
        )
        .expect("hashing is separate work");

    // Once the first job finishes it stops being outstanding, so the file may be
    // re-analysed after an extractor upgrade.
    connection
        .execute(
            "UPDATE analysis_jobs SET state = 'done' WHERE id = 'job-1'",
            [],
        )
        .expect("finishing the job");
    connection
        .execute(
            "INSERT INTO analysis_jobs (id, media_file_id, kind, created_at, updated_at)
             VALUES ('job-4', 'file-1', 'features', ?1, ?1)",
            [NOW],
        )
        .expect("re-analysis after the previous run finished");
}

#[test]
fn a_duplicate_review_must_say_what_it_duplicates() {
    let db = TempDb::new();
    let connection = db.pool().get().expect("a connection");

    insert_profile(&connection, "profile-1", "Sasha");
    insert_media_file(&connection, "file-1", "D:/Music/a.flac");
    insert_media_file(&connection, "file-2", "D:/Music/copy/a.flac");

    let err = connection
        .execute(
            "INSERT INTO import_review (id, profile_id, media_file_id, reason, created_at)
             VALUES ('review-1', 'profile-1', 'file-2', 'duplicate', ?1)",
            [NOW],
        )
        .expect_err("an unanswerable review entry must be rejected");
    assert!(is_constraint_violation(&err), "got {err}");

    connection
        .execute(
            "INSERT INTO import_review
                 (id, profile_id, media_file_id, duplicate_media_file_id, reason, created_at)
             VALUES ('review-2', 'profile-1', 'file-2', 'file-1', 'duplicate', ?1)",
            [NOW],
        )
        .expect("naming the other copy makes it answerable");
}
