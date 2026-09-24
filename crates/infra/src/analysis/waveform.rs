//! Measuring a track's waveform in the background, and keeping it.
//!
//! One thread, lowered in priority, taking files one at a time off a channel:
//! the whole file is decoded - the only way to know its shape - and reduced to
//! [`WAVEFORM_POINTS`] loudness readings, then kept in `track_waveforms`. A
//! file is measured once; after that its shape is a row.

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::ports::log::{LogLevel, LogPort};
use cadenza_core::domain::ports::repositories::MediaFileRepositoryPort;
use cadenza_core::domain::ports::system_priority::{PriorityClass, SystemPriorityPort};
use cadenza_core::domain::ports::waveform::WaveformPort;
use cadenza_core::domain::waveform::{WAVEFORM_POINTS, levels};
use cadenza_core::{CoreError, Result};

use crate::audio::symphonia_decoder::TrackStream;
use crate::db::SqlitePool;
use crate::db::error::db_error_in;

/// The waveform store, with the thread that fills it.
pub struct WaveformStore {
    pool: SqlitePool,
    requests: Mutex<Option<Sender<MediaFileId>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl WaveformStore {
    /// Starts the measuring thread. It stops when the store is dropped.
    pub fn start(
        pool: SqlitePool,
        media_files: Arc<dyn MediaFileRepositoryPort>,
        priority: Arc<dyn SystemPriorityPort>,
        log: Arc<dyn LogPort>,
    ) -> Self {
        let (sender, requests) = mpsc::channel::<MediaFileId>();
        let handle = thread::Builder::new()
            .name("cadenza-waveform".to_owned())
            .spawn({
                let pool = pool.clone();
                move || {
                    let _ = priority.set_current_thread(PriorityClass::Background);
                    // Asked for twice while the first is still being measured
                    // is still measured once.
                    let mut done = HashSet::new();
                    for id in requests {
                        if !done.insert(id) || stored(&pool, id).unwrap_or(false) {
                            continue;
                        }
                        if let Err(err) = measure_and_keep(&pool, media_files.as_ref(), id) {
                            log.write(LogLevel::Warn, &format!("no waveform for {id}: {err}"));
                        }
                    }
                }
            })
            .ok();

        Self {
            pool,
            requests: Mutex::new(Some(sender)),
            handle: Mutex::new(handle),
        }
    }
}

impl WaveformPort for WaveformStore {
    fn get(&self, media_file_id: MediaFileId) -> Result<Option<Vec<u8>>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare("SELECT heights FROM track_waveforms WHERE media_file_id = ?1")
            .map_err(db_error_in("reading a waveform"))?;
        let mut rows = statement
            .query_map([media_file_id.to_string()], |row| row.get::<_, Vec<u8>>(0))
            .map_err(db_error_in("reading a waveform"))?;
        rows.next()
            .transpose()
            .map_err(db_error_in("reading a waveform"))
    }

    fn request(&self, media_file_id: MediaFileId) {
        if let Some(sender) = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = sender.send(media_file_id);
        }
    }
}

impl Drop for WaveformStore {
    fn drop(&mut self) {
        // Closing the channel is what ends the thread's loop.
        drop(
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        if let Some(handle) = self
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = handle.join();
        }
    }
}

fn stored(pool: &SqlitePool, id: MediaFileId) -> Result<bool> {
    let connection = pool.get()?;
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM track_waveforms WHERE media_file_id = ?1)",
            [id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(db_error_in("checking for a waveform"))
}

fn measure_and_keep(
    pool: &SqlitePool,
    media_files: &dyn MediaFileRepositoryPort,
    id: MediaFileId,
) -> Result<()> {
    let file = media_files
        .get(id)?
        .ok_or_else(|| CoreError::not_found("media file", id))?;
    let shape = levels(&measure(&file.path)?);

    let connection = pool.get()?;
    connection
        .execute(
            "INSERT OR REPLACE INTO track_waveforms (media_file_id, heights) VALUES (?1, ?2)",
            rusqlite::params![id.to_string(), shape],
        )
        .map_err(db_error_in("keeping a waveform"))?;
    Ok(())
}

/// Loudness, as RMS, over [`WAVEFORM_POINTS`] equal stretches of a file.
pub fn measure(path: &Path) -> Result<Vec<f32>> {
    let mut stream = TrackStream::open(path)?;
    let info = stream.info();
    let channels = usize::from(info.channels).max(1);
    let total = info.duration.as_millis() * u64::from(info.sample_rate) / 1_000;
    if total == 0 {
        return Err(CoreError::Audio(
            "the file does not say how long it is".into(),
        ));
    }

    let mut sums = vec![0.0_f64; WAVEFORM_POINTS];
    let mut counts = vec![0_u64; WAVEFORM_POINTS];
    let mut frame = 0_u64;
    while let Some(samples) = stream.next_frames()? {
        for chunk in samples.chunks_exact(channels) {
            let bucket =
                ((frame * WAVEFORM_POINTS as u64 / total) as usize).min(WAVEFORM_POINTS - 1);
            let energy: f32 = chunk.iter().map(|sample| sample * sample).sum();
            sums[bucket] += f64::from(energy / channels as f32);
            counts[bucket] += 1;
            frame += 1;
        }
    }

    Ok(sums
        .iter()
        .zip(&counts)
        .map(|(sum, &count)| {
            if count == 0 {
                0.0
            } else {
                (sum / count as f64).sqrt() as f32
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::waveform::WAVEFORM_POINTS;
    use cadenza_testkit::TempDir;
    use cadenza_testkit::audio_fixtures::write_wav;

    use super::measure;

    #[test]
    fn a_steady_file_measures_the_same_level_all_the_way_along() {
        let directory = TempDir::new("waveform-steady");
        let path = write_wav(directory.path(), "tone.wav", 2, 16_000);

        let levels = measure(&path).expect("measured");
        assert_eq!(levels.len(), WAVEFORM_POINTS);
        let level = 16_000.0 / 32_768.0;
        assert!(
            levels.iter().all(|reading| (reading - level).abs() < 0.01),
            "every stretch reads the file's one level: {levels:?}"
        );
    }
}
