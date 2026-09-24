//! The preset a listener has chosen for one track.
//!
//! **This does not replace `profile_settings`**, which still holds what the
//! filters are actually doing — a preset with a control nudged afterwards is no
//! longer that preset. This table says something narrower and longer-lived:
//! *this listener wants this track played this way*, stored as a pointer at a
//! preset rather than a curve, because a preset is what somebody chooses.
//!
//! One row per track at most, and the profile is half the key: two listeners
//! sharing a machine share the file and not the opinion of it.
//!
//! Everything cascades — a deleted profile, a file leaving the catalogue or a
//! deleted preset all take these rows with them, since what is left would name
//! something that no longer exists.

pub const SQL: &str = r#"
CREATE TABLE profile_track_eq (
    profile_id    TEXT    NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    media_file_id TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,
    preset_id     TEXT    NOT NULL REFERENCES eq_presets (id) ON DELETE CASCADE,
    updated_at    INTEGER NOT NULL,

    PRIMARY KEY (profile_id, media_file_id)
) STRICT;

-- What the cascade above needs to find, and what "which tracks use this
-- preset" would ask for if anything ever does.
CREATE INDEX profile_track_eq_preset ON profile_track_eq (preset_id);
"#;
