//! The preset a listener has chosen for one track.
//!
//! The equaliser used to be one sound for everything, which is the setting the
//! table below does not replace: `profile_settings` still holds what the
//! filters are doing, because a preset with a control nudged afterwards is no
//! longer any preset at all. This says something narrower and longer-lived —
//! *this listener wants this track played this way* — and it says it as a
//! pointer at a preset rather than as a curve, because that is what somebody
//! chooses.
//!
//! One row per track at most, and the profile is half the key: two listeners
//! sharing a machine share the file and not the opinion of it.
//!
//! Everything cascades. A profile deleted takes its choices with it, a file
//! that leaves the catalogue takes the choice made about it, and a preset
//! deleted takes every track that pointed at it — which is the right answer for
//! all three: what is left would otherwise be a row naming something that no
//! longer exists.

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
