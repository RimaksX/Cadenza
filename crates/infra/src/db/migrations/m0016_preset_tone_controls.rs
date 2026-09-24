//! The tone-control half of the presets that shipped with eight bands.
//!
//! Migration 15 gave every preset but Flat a parametric curve and left its
//! three tone controls at zero, because a preset carried a mode and applying
//! one switched to it. That was wrong in the way the listener found first:
//! choosing "Rock" in the simple mode threw them into the advanced one, and
//! choosing it *for* the simple mode did nothing at all.
//!
//! A preset now describes the same intention twice — as eight bands and as
//! three controls — and applying one leaves the listener where they are. These
//! are the three-control readings of the curves in migration 15: the same
//! shape, said in the vocabulary of a bass, a middle and a treble.

pub const SQL: &str = r#"
UPDATE eq_presets SET simple_bass_gain = 2.0,  simple_mid_gain = -1.0, simple_treble_gain = 2.5
    WHERE id = '00000000-0000-4000-8000-000000000002';  -- Pop
UPDATE eq_presets SET simple_bass_gain = 4.0,  simple_mid_gain = -1.5, simple_treble_gain = 3.0
    WHERE id = '00000000-0000-4000-8000-000000000003';  -- Rock
UPDATE eq_presets SET simple_bass_gain = 1.5,  simple_mid_gain = 0.0,  simple_treble_gain = 2.0
    WHERE id = '00000000-0000-4000-8000-000000000004';  -- Classical
UPDATE eq_presets SET simple_bass_gain = 5.0,  simple_mid_gain = -2.0, simple_treble_gain = 4.0
    WHERE id = '00000000-0000-4000-8000-000000000005';  -- Electronic
UPDATE eq_presets SET simple_bass_gain = -3.0, simple_mid_gain = 3.5,  simple_treble_gain = 0.0
    WHERE id = '00000000-0000-4000-8000-000000000006';  -- Vocal Enhance
UPDATE eq_presets SET simple_bass_gain = 2.5,  simple_mid_gain = -2.0, simple_treble_gain = 3.5
    WHERE id = '00000000-0000-4000-8000-000000000007';  -- Spatial Enhance
UPDATE eq_presets SET simple_bass_gain = 8.0,  simple_mid_gain = 1.0,  simple_treble_gain = 0.0
    WHERE id = '00000000-0000-4000-8000-000000000008';  -- Bass Boost
UPDATE eq_presets SET simple_bass_gain = 0.0,  simple_mid_gain = 0.0,  simple_treble_gain = 6.0
    WHERE id = '00000000-0000-4000-8000-000000000009';  -- Treble Boost
"#;
