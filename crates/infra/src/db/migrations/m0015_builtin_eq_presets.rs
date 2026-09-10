//! The nine equaliser presets shipped with the application.
//!
//! They are named but their curves are not specified anywhere, so the
//! curves are ours. They are written here rather than seeded by the application
//! for the reason every migration exists: a database that has run this has
//! them, and one that has not is not a database this version talks to. Nothing
//! has to remember to insert them, and nothing can insert them twice.
//!
//! Eight of the nine are parametric, because a preset is a
//! curve and a parametric band can be put where the curve wants it rather than
//! at the nearest of ten fixed points. Flat is the exception: it is the sound
//! of the equaliser doing nothing, and it belongs in the simple mode where
//! doing nothing is what the three controls are already set to.
//!
//! The identifiers are literal and fixed. A built-in preset is the same preset
//! on every machine, which is what lets a profile point at one.
//!
//! Timestamps are zero: these did not happen at a moment, they shipped.

pub const SQL: &str = r#"
INSERT INTO eq_presets
    (id, profile_id, name, is_builtin, mode,
     simple_bass_gain, simple_mid_gain, simple_treble_gain,
     advanced_bands_json, created_at, updated_at)
VALUES
    ('00000000-0000-4000-8000-000000000001', NULL, 'Flat', 1, 'simple',
     0, 0, 0, NULL, 0, 0),

    ('00000000-0000-4000-8000-000000000002', NULL, 'Pop', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":60,"q":1.0,"gain_db":2.0},
       {"frequency_hz":150,"q":1.0,"gain_db":1.0},
       {"frequency_hz":400,"q":1.2,"gain_db":-1.5},
       {"frequency_hz":1000,"q":1.0,"gain_db":-1.0},
       {"frequency_hz":2500,"q":1.0,"gain_db":1.5},
       {"frequency_hz":6000,"q":1.0,"gain_db":3.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":16000,"q":1.0,"gain_db":1.0}]', 0, 0),

    ('00000000-0000-4000-8000-000000000003', NULL, 'Rock', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":60,"q":1.0,"gain_db":4.0},
       {"frequency_hz":150,"q":1.0,"gain_db":2.0},
       {"frequency_hz":400,"q":1.2,"gain_db":-2.0},
       {"frequency_hz":1000,"q":1.0,"gain_db":-1.0},
       {"frequency_hz":2500,"q":1.0,"gain_db":2.0},
       {"frequency_hz":6000,"q":1.0,"gain_db":3.5},
       {"frequency_hz":10000,"q":1.0,"gain_db":2.5},
       {"frequency_hz":16000,"q":1.0,"gain_db":1.5}]', 0, 0),

    ('00000000-0000-4000-8000-000000000004', NULL, 'Classical', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":60,"q":1.0,"gain_db":1.5},
       {"frequency_hz":150,"q":1.0,"gain_db":0.5},
       {"frequency_hz":400,"q":1.0,"gain_db":0.0},
       {"frequency_hz":1000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":2500,"q":1.0,"gain_db":-0.5},
       {"frequency_hz":6000,"q":1.0,"gain_db":1.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":16000,"q":1.0,"gain_db":2.5}]', 0, 0),

    ('00000000-0000-4000-8000-000000000005', NULL, 'Electronic', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":50,"q":1.0,"gain_db":5.0},
       {"frequency_hz":150,"q":1.0,"gain_db":3.0},
       {"frequency_hz":400,"q":1.2,"gain_db":-2.0},
       {"frequency_hz":1000,"q":1.0,"gain_db":-2.0},
       {"frequency_hz":2500,"q":1.0,"gain_db":0.0},
       {"frequency_hz":6000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":4.0},
       {"frequency_hz":16000,"q":1.0,"gain_db":4.0}]', 0, 0),

    ('00000000-0000-4000-8000-000000000006', NULL, 'Vocal Enhance', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":60,"q":1.0,"gain_db":-3.0},
       {"frequency_hz":150,"q":1.0,"gain_db":-1.5},
       {"frequency_hz":400,"q":1.0,"gain_db":0.5},
       {"frequency_hz":1000,"q":1.2,"gain_db":3.0},
       {"frequency_hz":2500,"q":1.2,"gain_db":4.0},
       {"frequency_hz":6500,"q":1.5,"gain_db":-1.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":1.0},
       {"frequency_hz":16000,"q":1.0,"gain_db":0.0}]', 0, 0),

    ('00000000-0000-4000-8000-000000000007', NULL, 'Spatial Enhance', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":60,"q":1.0,"gain_db":2.5},
       {"frequency_hz":150,"q":1.0,"gain_db":0.0},
       {"frequency_hz":400,"q":1.0,"gain_db":-1.5},
       {"frequency_hz":1000,"q":1.0,"gain_db":-2.0},
       {"frequency_hz":2500,"q":1.0,"gain_db":-0.5},
       {"frequency_hz":6000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":3.5},
       {"frequency_hz":16000,"q":1.0,"gain_db":4.0}]', 0, 0),

    ('00000000-0000-4000-8000-000000000008', NULL, 'Bass Boost', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":40,"q":0.9,"gain_db":8.0},
       {"frequency_hz":80,"q":1.0,"gain_db":6.0},
       {"frequency_hz":150,"q":1.0,"gain_db":4.0},
       {"frequency_hz":300,"q":1.2,"gain_db":1.5},
       {"frequency_hz":1000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":2500,"q":1.0,"gain_db":0.0},
       {"frequency_hz":6000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":0.0}]', 0, 0),

    ('00000000-0000-4000-8000-000000000009', NULL, 'Treble Boost', 1, 'advanced',
     0, 0, 0,
     '[{"frequency_hz":60,"q":1.0,"gain_db":0.0},
       {"frequency_hz":150,"q":1.0,"gain_db":0.0},
       {"frequency_hz":400,"q":1.0,"gain_db":0.0},
       {"frequency_hz":1000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":3000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":6000,"q":1.0,"gain_db":4.0},
       {"frequency_hz":10000,"q":1.0,"gain_db":6.0},
       {"frequency_hz":16000,"q":1.0,"gain_db":7.0}]', 0, 0);
"#;
