//! Saving a copy of everything, and putting one back.

use std::path::PathBuf;
use std::sync::Arc;

use crate::Result;
use crate::application::context::AppContext;
use crate::domain::ports::backup::BackupPort;
use crate::domain::ports::folder_picker::{FileKind, FolderPickerPort};

/// What the backup use cases need.
pub struct BackupPorts {
    pub picker: Arc<dyn FolderPickerPort>,
    pub backup: Arc<dyn BackupPort>,
}

/// Saving a copy of the listeners' data and restoring one.
pub struct BackupService {
    context: Arc<AppContext>,
    ports: BackupPorts,
}

impl BackupService {
    #[must_use]
    pub fn new(context: Arc<AppContext>, ports: BackupPorts) -> Self {
        Self { context, ports }
    }

    /// Asks where, and saves a copy there. `None` when nothing was chosen.
    pub fn save_copy(&self) -> Result<Option<PathBuf>> {
        let suggested = format!("Cadenza {}.cadenza", date(self.context.now().as_millis()));
        let Some(to) =
            self.ports
                .picker
                .save_file("Save a copy of Cadenza", FileKind::Backup, &suggested)?
        else {
            return Ok(None);
        };
        self.ports.backup.save(&to)?;
        Ok(Some(to))
    }

    /// Asks which copy, checks it, and sets it to replace everything on the
    /// next start. `false` when nothing was chosen.
    pub fn restore(&self) -> Result<bool> {
        let Some(from) = self
            .ports
            .picker
            .pick_file("Restore Cadenza from a copy", FileKind::Backup)?
        else {
            return Ok(false);
        };
        self.ports.backup.stage_restore(&from)?;
        Ok(true)
    }
}

/// `YYYY-MM-DD` for a moment in milliseconds since 1970, in UTC.
fn date(millis: i64) -> String {
    // Days to a civil date, after Howard Hinnant's `civil_from_days`.
    let days = millis.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::date;

    #[test]
    fn a_moment_is_named_by_its_day() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400_000), "2000-02-29");
        // 2026-09-22 13:00 UTC.
        assert_eq!(date(1_790_082_000_000), "2026-09-22");
    }
}
