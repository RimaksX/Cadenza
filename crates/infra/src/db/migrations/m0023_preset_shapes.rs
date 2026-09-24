//! New curves for five presets that measured as one curve at five volumes.
//!
//! Pop against Rock was 0.6 dB apart once their levels were matched, with a
//! shape correlation of 0.97; Pop, Rock, Classical, Electronic and Spatial all
//! correlated 0.69 to 0.97 with each other. Each now has a shape of its own
//! rather than a share of the same smile. Vocal Enhance, Bass Boost and Treble
//! Boost already measured apart and are not touched.
//!
//! The three tone controls are fitted, not guessed: for each curve a search
//! over the simple mode's shelf-bell-shelf found the closest (bass, mid,
//! treble), so a preset still says the same thing in both modes.
//!
//! **Migration 15 is left exactly as it shipped.** It has run on machines that
//! are not ours, and the only way to change what a migration did is another
//! migration saying so.

pub const SQL: &str = r#"
UPDATE eq_presets SET
    simple_bass_gain = 0.5, simple_mid_gain = 2.0, simple_treble_gain = 2.0,
    advanced_bands_json =
     '[{"frequency_hz":45,"q":1.0,"gain_db":0.0},
       {"frequency_hz":120,"q":1.0,"gain_db":1.5},
       {"frequency_hz":300,"q":1.2,"gain_db":-1.0},
       {"frequency_hz":800,"q":1.0,"gain_db":0.0},
       {"frequency_hz":2000,"q":1.2,"gain_db":2.0},
       {"frequency_hz":3800,"q":1.2,"gain_db":4.0},
       {"frequency_hz":7000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":13000,"q":1.0,"gain_db":0.5}]'
    WHERE id = '00000000-0000-4000-8000-000000000002';  -- Pop

UPDATE eq_presets SET
    simple_bass_gain = 2.5, simple_mid_gain = -0.5, simple_treble_gain = -0.5,
    advanced_bands_json =
     '[{"frequency_hz":70,"q":1.0,"gain_db":4.0},
       {"frequency_hz":160,"q":1.0,"gain_db":1.5},
       {"frequency_hz":400,"q":1.4,"gain_db":-3.5},
       {"frequency_hz":900,"q":1.0,"gain_db":-1.0},
       {"frequency_hz":2500,"q":1.2,"gain_db":2.5},
       {"frequency_hz":4500,"q":1.2,"gain_db":3.5},
       {"frequency_hz":8000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":15000,"q":1.0,"gain_db":-2.0}]'
    WHERE id = '00000000-0000-4000-8000-000000000003';  -- Rock

UPDATE eq_presets SET
    simple_bass_gain = 1.0, simple_mid_gain = 0.0, simple_treble_gain = 2.0,
    advanced_bands_json =
     '[{"frequency_hz":40,"q":1.0,"gain_db":1.5},
       {"frequency_hz":100,"q":1.0,"gain_db":0.5},
       {"frequency_hz":250,"q":1.0,"gain_db":0.0},
       {"frequency_hz":800,"q":1.0,"gain_db":0.0},
       {"frequency_hz":2000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":5000,"q":1.0,"gain_db":0.0},
       {"frequency_hz":11000,"q":1.0,"gain_db":1.0},
       {"frequency_hz":17000,"q":1.0,"gain_db":2.0}]'
    WHERE id = '00000000-0000-4000-8000-000000000004';  -- Classical

UPDATE eq_presets SET
    simple_bass_gain = 6.5, simple_mid_gain = -4.0, simple_treble_gain = 8.0,
    advanced_bands_json =
     '[{"frequency_hz":35,"q":1.0,"gain_db":6.5},
       {"frequency_hz":80,"q":1.0,"gain_db":4.5},
       {"frequency_hz":350,"q":1.2,"gain_db":-3.5},
       {"frequency_hz":1200,"q":1.2,"gain_db":-3.5},
       {"frequency_hz":3000,"q":1.0,"gain_db":-1.5},
       {"frequency_hz":7000,"q":1.0,"gain_db":2.0},
       {"frequency_hz":12000,"q":1.0,"gain_db":5.0},
       {"frequency_hz":16000,"q":1.0,"gain_db":5.0}]'
    WHERE id = '00000000-0000-4000-8000-000000000005';  -- Electronic

UPDATE eq_presets SET
    simple_bass_gain = -0.5, simple_mid_gain = -4.0, simple_treble_gain = 3.0,
    advanced_bands_json =
     '[{"frequency_hz":50,"q":1.0,"gain_db":0.0},
       {"frequency_hz":150,"q":1.0,"gain_db":-1.0},
       {"frequency_hz":400,"q":1.4,"gain_db":-4.0},
       {"frequency_hz":1000,"q":1.2,"gain_db":-3.5},
       {"frequency_hz":2500,"q":1.0,"gain_db":-0.5},
       {"frequency_hz":5000,"q":1.0,"gain_db":1.5},
       {"frequency_hz":9000,"q":1.2,"gain_db":3.5},
       {"frequency_hz":16000,"q":1.0,"gain_db":0.5}]'
    WHERE id = '00000000-0000-4000-8000-000000000007';  -- Spatial Enhance
"#;
