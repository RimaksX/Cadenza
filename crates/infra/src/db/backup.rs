//! A copy of everything Cadenza keeps, as one file, and putting one back.
//!
//! The copy is a SQLite database: the live one as `VACUUM INTO` writes it,
//! with two tables added - `backup_meta`, which says what it is, and
//! `backup_pictures`, the pictures listeners chose, which live beside the
//! database rather than in it. One file that anything reading SQLite can
//! open, and no archive format to depend on.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use cadenza_core::domain::ports::backup::BackupPort;
use cadenza_core::{CoreError, Result};
use rusqlite::{Connection, OpenFlags};

use super::SqlitePool;
use super::error::db_error_in;
use super::migrations::LATEST_VERSION;

/// What a copy written by this version says it is. Raised when the shape of
/// the added tables changes.
const FORMAT: &str = "1";

/// The pictures nothing could make again: covers chosen for tracks and
/// playlists, and listeners' faces. A cover read out of a music file is left
/// out, because the file still has it.
const KEPT: [&str; 3] = ["chosen-", "playlist-", "profile-"];

fn is_kept_picture(name: &str) -> bool {
    KEPT.iter().any(|prefix| name.starts_with(prefix))
        // The smaller copies are made again from the original.
        && !name.contains(".thumb.")
        && !name.contains(".tile.")
        // A name out of a file somebody handed over: never a path.
        && !name.contains(['/', '\\'])
        && !name.contains("..")
}

/// Saves and stages copies of the database behind `pool`.
pub struct SqliteBackup {
    pool: SqlitePool,
    database: PathBuf,
    artwork: PathBuf,
}

impl SqliteBackup {
    #[must_use]
    pub fn new(pool: SqlitePool, database: PathBuf, artwork: PathBuf) -> Self {
        Self {
            pool,
            database,
            artwork,
        }
    }
}

/// Where a copy waits to replace the data on the next start.
#[must_use]
pub fn staged_path(database: &Path) -> PathBuf {
    sibling(database, ".restore")
}

fn sibling(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn storage(what: &str, err: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{what}: {err}"))
}

impl BackupPort for SqliteBackup {
    fn save(&self, to: &Path) -> Result<()> {
        // Written beside and then moved into place, so a copy that failed
        // halfway never stands where a good one of the same name did.
        let partial = to.with_extension("cadenza-partial");
        let _ = fs::remove_file(&partial);

        let written = (|| {
            self.pool
                .get()?
                .execute("VACUUM INTO ?1", [partial.to_string_lossy()])
                .map_err(db_error_in("copying the database"))?;

            let copy = Connection::open(&partial).map_err(db_error_in("opening the copy"))?;
            copy.execute_batch(
                "CREATE TABLE backup_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE backup_pictures (name TEXT PRIMARY KEY, bytes BLOB NOT NULL);",
            )
            .map_err(db_error_in("marking the copy"))?;
            copy.execute(
                "INSERT INTO backup_meta (key, value) VALUES ('format', ?1)",
                [FORMAT],
            )
            .map_err(db_error_in("marking the copy"))?;

            let entries =
                fs::read_dir(&self.artwork).map_err(|err| storage("reading the pictures", err))?;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !is_kept_picture(&name) {
                    continue;
                }
                let bytes =
                    fs::read(entry.path()).map_err(|err| storage("reading a picture", err))?;
                copy.execute(
                    "INSERT INTO backup_pictures (name, bytes) VALUES (?1, ?2)",
                    (&name, &bytes),
                )
                .map_err(db_error_in("keeping a picture"))?;
            }
            Ok::<_, CoreError>(())
        })();

        if let Err(err) = written {
            let _ = fs::remove_file(&partial);
            return Err(err);
        }
        fs::rename(&partial, to).map_err(|err| storage("saving the copy", err))
    }

    fn stage_restore(&self, from: &Path) -> Result<()> {
        check(from)?;
        fs::copy(from, staged_path(&self.database))
            .map(drop)
            .map_err(|err| storage("setting the copy aside", err))
    }
}

/// Is `path` a copy this version can take: one of ours, whole, and not made
/// by a newer Cadenza whose database this one cannot read.
fn check(path: &Path) -> Result<()> {
    let refuse = |why: &str| CoreError::invalid("copy", why);
    let copy = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| refuse("this file is not a copy of Cadenza"))?;

    let format: String = copy
        .query_row(
            "SELECT value FROM backup_meta WHERE key = 'format'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| refuse("this file is not a copy of Cadenza"))?;
    if format != FORMAT {
        return Err(refuse("this copy was made by a newer Cadenza"));
    }

    let schema: i64 = copy
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .map_err(|_| refuse("this file is not a copy of Cadenza"))?;
    if schema > LATEST_VERSION {
        return Err(refuse("this copy was made by a newer Cadenza"));
    }

    let whole: String = copy
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|_| refuse("this copy is damaged"))?;
    if whole != "ok" {
        return Err(refuse("this copy is damaged"));
    }
    Ok(())
}

/// Moves a file, and tries again for a moment when Windows says it is in use:
/// the process that had the database open may still be on its way out.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(_) if tries < 30 => {
                tries += 1;
                thread::sleep(Duration::from_millis(100));
            }
            done => return done,
        }
    }
}

/// Puts a staged copy in place of the data, before anything has it open.
///
/// What it replaces is kept beside it as `<database>.previous`, journal and
/// all, until the next restore: a copy put back by mistake is not the end of
/// what was there. A copy that does not check out is set aside as
/// `<database>.rejected` rather than tried again at every start.
///
/// `Ok(None)` when nothing was waiting; otherwise what happened, for the log.
pub fn apply_staged_restore(database: &Path, artwork: &Path) -> Result<Option<String>> {
    let staged = staged_path(database);
    if !staged.exists() {
        return Ok(None);
    }
    if let Err(err) = check(&staged) {
        let _ = move_file(&staged, &sibling(database, ".rejected"));
        return Err(err);
    }

    const PARTS: [&str; 3] = ["", "-wal", "-shm"];
    let previous = sibling(database, ".previous");
    for part in PARTS {
        let _ = fs::remove_file(sibling(&previous, part));
    }
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for part in PARTS {
        let from = sibling(database, part);
        if from.exists() {
            let to = sibling(&previous, part);
            if let Err(err) = move_file(&from, &to) {
                for (to, from) in moved {
                    let _ = move_file(&to, &from);
                }
                return Err(storage("setting the current data aside", err));
            }
            moved.push((to, from));
        }
    }
    if let Err(err) = move_file(&staged, database) {
        for (to, from) in moved {
            let _ = move_file(&to, &from);
        }
        return Err(storage("putting the copy in place", err));
    }

    // The pictures: the ones kept now go, the copy's come back, and the
    // tables that carried them leave the database.
    let copy = Connection::open(database).map_err(db_error_in("opening the restored data"))?;
    if let Ok(entries) = fs::read_dir(artwork) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if KEPT.iter().any(|prefix| name.starts_with(prefix)) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let mut pictures = 0;
    {
        let mut statement = copy
            .prepare("SELECT name, bytes FROM backup_pictures")
            .map_err(db_error_in("reading the copy's pictures"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(db_error_in("reading the copy's pictures"))?;
        for (name, bytes) in rows.flatten() {
            if is_kept_picture(&name) && fs::write(artwork.join(&name), bytes).is_ok() {
                pictures += 1;
            }
        }
    }
    copy.execute_batch("DROP TABLE backup_pictures; DROP TABLE backup_meta;")
        .map_err(db_error_in("clearing the copy's own tables"))?;

    Ok(Some(format!(
        "restored from a copy, with {pictures} pictures; the data it replaced is at {}",
        previous.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pictures_nothing_could_make_again_are_kept() {
        assert!(is_kept_picture("chosen-a-b.png"));
        assert!(is_kept_picture("playlist-x.jpg"));
        assert!(is_kept_picture("profile-y.png"));
        assert!(!is_kept_picture("0b1c.png"));
        assert!(!is_kept_picture("profile-y.thumb.png"));
        assert!(!is_kept_picture("playlist-x.tile.png"));
        assert!(!is_kept_picture("profile-../../evil.png"));
        assert!(!is_kept_picture("chosen-a\\b.png"));
    }

    #[test]
    fn a_copy_saved_and_put_back_restores_the_data_and_the_pictures() {
        let root = std::env::temp_dir().join(format!("cadenza-backup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let artwork = root.join("artwork");
        fs::create_dir_all(&artwork).unwrap();
        let database = root.join("app.db");

        let pool = super::super::open(&database).unwrap();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value_json, updated_at) VALUES ('marker', '1', 0)",
                [],
            )
            .unwrap();
        fs::write(artwork.join("profile-me.png"), b"face").unwrap();
        fs::write(artwork.join("track.png"), b"from the file").unwrap();

        let backup = SqliteBackup::new(pool.clone(), database.clone(), artwork.clone());
        let copy = root.join("copy.cadenza");
        backup.save(&copy).unwrap();

        // Things change after the copy was made.
        pool.get()
            .unwrap()
            .execute("DELETE FROM app_settings WHERE key = 'marker'", [])
            .unwrap();
        fs::write(artwork.join("profile-me.png"), b"another face").unwrap();

        backup.stage_restore(&copy).unwrap();
        drop(backup);
        drop(pool);
        let said = apply_staged_restore(&database, &artwork).unwrap();
        assert!(said.is_some());

        let restored = super::super::open(&database).unwrap();
        let marker: i64 = restored
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key = 'marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, 1);
        assert_eq!(fs::read(artwork.join("profile-me.png")).unwrap(), b"face");
        assert_eq!(
            fs::read(artwork.join("track.png")).unwrap(),
            b"from the file"
        );
        assert!(sibling(&database, ".previous").exists());
        assert!(!staged_path(&database).exists());
        drop(restored);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_is_not_a_copy_is_refused() {
        let root = std::env::temp_dir().join(format!("cadenza-notcopy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("x.cadenza");
        fs::write(&file, b"not a database").unwrap();
        assert!(check(&file).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
