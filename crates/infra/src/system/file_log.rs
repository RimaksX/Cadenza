//! The log file, and the only thing that writes it.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use cadenza_core::Result;
use cadenza_core::domain::ports::clock::ClockPort;
use cadenza_core::domain::ports::log::{LogLevel, LogPort};
use cadenza_core::domain::value_objects::Timestamp;

/// How large the log may get before it is rolled over.
///
/// A calibration knob. Big enough to hold a long session of a chatty scan,
/// small enough that somebody asked to send it can. One previous generation is
/// kept, which is what "what happened last time I ran it" needs and no more.
const MAX_BYTES: u64 = 1024 * 1024;

/// Appends lines to `%LOCALAPPDATA%/Cadenza/logs/app.log`.
///
/// Under a lock, because the whole point is the threads: a watcher, an analyser
/// and the window all write here, and two half-lines interleaved is a log that
/// costs more to read than it saves.
pub struct FileLog {
    path: PathBuf,
    previous: PathBuf,
    clock: Arc<dyn ClockPort>,
    file: Mutex<Option<File>>,
}

impl FileLog {
    /// Opens the log, creating its directory.
    pub fn new(path: PathBuf, previous: PathBuf, clock: Arc<dyn ClockPort>) -> Result<Self> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory).map_err(|err| {
                cadenza_core::CoreError::FileSystem(format!(
                    "could not create the log directory {}: {err}",
                    directory.display()
                ))
            })?;
        }

        Ok(Self {
            path,
            previous,
            clock,
            file: Mutex::new(None),
        })
    }

    /// Rolls the log over when it has grown past [`MAX_BYTES`].
    ///
    /// Checked on open rather than on every line: a run that starts with a full
    /// log gets a fresh one, and a run that fills one keeps writing until it
    /// ends. What that costs is a log slightly over the limit; what it saves is
    /// a `stat` per line.
    fn open(&self) -> Option<File> {
        if std::fs::metadata(&self.path).is_ok_and(|data| data.len() >= MAX_BYTES) {
            let _ = std::fs::rename(&self.path, &self.previous);
        }

        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok()
    }
}

impl LogPort for FileLog {
    fn write(&self, level: LogLevel, message: &str) {
        let line = format!(
            "{} {:<5} {}\n",
            stamp(self.clock.now()),
            level.as_str(),
            message
        );

        let mut held = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if held.is_none() {
            *held = self.open();
        }

        // A log that cannot be written is not worth a second failure. Whatever
        // was being reported has already happened, and the application is still
        // playing music.
        if let Some(file) = held.as_mut() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

/// `2026-08-19 14:05:09` in local-looking form, from unix milliseconds.
///
/// Computed here rather than taken from a date library, because this is the
/// only place in the application that needs a civil date and one function is
/// cheaper than a dependency. The algorithm is Howard Hinnant's `civil_from_days`,
/// which is the one everybody's date library is built on.
fn stamp(now: Timestamp) -> String {
    let millis = now.as_millis();
    let seconds = millis.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60,
    );

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Days since the unix epoch to a civil year, month and day.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;

    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{FileLog, MAX_BYTES, stamp};
    use cadenza_core::domain::ports::log::{LogLevel, LogPort};
    use cadenza_core::domain::value_objects::Timestamp;
    use cadenza_testkit::TestClock;

    fn temporary(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cadenza-log-{}-{tag}", std::process::id()))
    }

    #[test]
    fn a_line_carries_the_time_the_level_and_the_message() {
        let root = temporary("lines");
        let _ = std::fs::remove_dir_all(&root);
        let clock = Arc::new(TestClock::at(1_755_600_309_000));

        let log =
            FileLog::new(root.join("app.log"), root.join("app.old.log"), clock).expect("a log");
        log.write(LogLevel::Warn, "a listen would not record");
        log.write(LogLevel::Error, "the folder could not be watched");

        let written = std::fs::read_to_string(root.join("app.log")).expect("reading");
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].ends_with("WARN  a listen would not record"),
            "{}",
            lines[0]
        );
        assert!(lines[0].starts_with("2025-08-19 "), "{}", lines[0]);
        assert!(lines[1].contains("ERROR the folder could not be watched"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_full_log_is_rolled_over_rather_than_grown() {
        let root = temporary("rollover");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a directory");

        let path = root.join("app.log");
        let previous = root.join("app.old.log");
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize]).expect("a full log");

        let log = FileLog::new(
            path.clone(),
            previous.clone(),
            Arc::new(TestClock::default()),
        )
        .expect("a log");
        log.write(LogLevel::Info, "a new run");

        assert_eq!(
            std::fs::metadata(&previous)
                .expect("the old one was kept")
                .len(),
            MAX_BYTES,
            "the previous log is the one that was full"
        );
        let written = std::fs::read_to_string(&path).expect("reading");
        assert!(written.ends_with("INFO  a new run\n"), "{written}");
        assert!(written.len() < 200, "the new log starts empty");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_stamp_is_a_civil_date() {
        // 2026-08-19 14:05:09 UTC, and a leap day, and the epoch itself.
        assert_eq!(
            stamp(Timestamp::from_millis(1_787_148_309_000)),
            "2026-08-19 14:05:09"
        );
        assert_eq!(
            stamp(Timestamp::from_millis(1_709_164_800_000)),
            "2024-02-29 00:00:00"
        );
        assert_eq!(stamp(Timestamp::from_millis(0)), "1970-01-01 00:00:00");
    }
}
