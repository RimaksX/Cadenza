//! Five presets that were one curve at five volumes.
//!
//! Measured through the application's own biquads: an impulse pushed through
//! the eight-band cascade, the response read at third-octave centres, and every
//! pair compared twice - once as it stands, and once with each curve's mean
//! level removed, because a listener undoes a difference in overall loudness
//! with the volume knob and only the *shape* survives that.
//!
//! Pop against Rock came back at 0.6 dB with the levels matched and a shape
//! correlation of **0.97**. That is not two presets. Pop, Rock, Classical,
//! Electronic and Spatial all correlated between 0.69 and 0.97 with each other:
//! one smile-shaped curve - bass up, middle down, treble up - drawn five times
//! at five amplitudes. Ten of the twenty-eight pairs sat under 2 dB
//! (`MASTER_ISSUES` 164).
//!
//! Each of the five now has a shape of its own, and the shapes are what the
//! names mean rather than how loud they are:
//!
//! - **Pop** - presence. A voice pushed forward around 2-4 kHz and almost
//!   nothing done anywhere else.
//! - **Rock** - punch and guitar edge, the mud at 400 Hz cut hard, and no
//!   sparkle on top at all.
//! - **Classical** - the only preset that cuts no band. A little weight low
//!   down, a little air, and otherwise the record as it was made.
//! - **Electronic** - the smile, taken all the way, because this is the one
//!   name the shape actually belongs to.
//! - **Spatial Enhance** - air, and room made for it by clearing 400-1000 Hz.
//!   No bass lift, which is what keeps it away from Electronic; a peak at 9 kHz
//!   that falls again by 16 kHz, which is what keeps it away from Treble Boost.
//!
//! Vocal Enhance, Bass Boost and Treble Boost are untouched: they were already
//! the three that measured apart from everything.
//!
//! The three tone controls are refitted rather than guessed. For each new curve
//! a search over the simple mode's shelf-bell-shelf found the (bass, mid,
//! treble) whose response sits closest to the eight bands, so a preset still
//! says the same thing twice and choosing one leaves the listener in whichever
//! mode they are in (migration 16).
//!
//! Migration 15 is left exactly as it shipped, for the reason 19 gives: it has
//! run on machines that are not ours, and the only way to change what it did is
//! another migration saying so.

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
