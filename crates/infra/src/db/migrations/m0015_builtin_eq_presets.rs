//! The nine equaliser presets shipped with the application.
//!
//! Only the names were given, so the curves are ours.
//!
//! **Five of these curves no longer apply: migration 23 replaces Pop, Rock,
//! Classical, Electronic and Spatial**, which measured as one curve at five
//! volumes. What stands here is what this migration did, not what a database
//! ends up with.
//!
//! Eight of the nine are parametric, because a band can then go where the curve
//! wants it rather than at the nearest of ten fixed points. Flat is the
//! exception: the sound of the equaliser doing nothing belongs in the simple
//! mode, where doing nothing is what the three controls already do.
//!
//! Identifiers are fixed so a profile can point at one; timestamps are zero
//! because these shipped rather than happened.

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
