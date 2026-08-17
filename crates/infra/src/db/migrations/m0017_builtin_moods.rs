//! The eight moods shipped with the application.
//!
//! PROJECT_MASTER 2.7 names them — Workout, Focus, Chill, Party, Driving,
//! Sleep, Gaming, Morning — and says nothing whatever about what they mean.
//! These bands are ours, and they are the part of radio most worth arguing
//! with: they are a claim about what music somebody wants while running, and
//! that claim is not a measurement.
//!
//! Each rule is a triple: the bottom of the band, the top, and how far outside
//! it a track can be before it stops counting. Soft edges rather than filters,
//! because a hard cut-off in a library of two hundred tracks is a mood that
//! produces nothing (`MoodRules` in `cadenza-core`).
//!
//! What the bands leave out is deliberate. Focus says nothing about valence —
//! concentrating is not a happy or a sad activity — and Driving says nothing
//! about danceability. A mood states what it cares about and stays quiet about
//! the rest, which is what keeps a small library from being scored against
//! opinions nobody holds.
//!
//! Written here rather than seeded by the application, for the reason every
//! migration exists: a database that has run this has them, nothing has to
//! remember to insert them, and nothing can insert them twice. Identifiers are
//! literal and fixed, so a session can point at one. Timestamps are zero:
//! these did not happen at a moment, they shipped.

pub const SQL: &str = r#"
INSERT INTO mood_presets
    (id, profile_id, name, is_builtin, feature_rules_json,
     genre_boost_json, ranking_weights_json, transition_profile,
     created_at, updated_at)
VALUES
    -- Running, lifting, moving. Fast and loud, and it must pull.
    ('00000000-0000-4000-9000-000000000001', NULL, 'Workout', 1,
     '{"bpm":[125,175,25],"energy":[0.65,1.0,0.25],"danceability":[0.55,1.0,0.3]}',
     NULL, NULL, 'crossfade', 0, 0),

    -- Working. Quiet and steady; nothing that asks to be listened to.
    ('00000000-0000-4000-9000-000000000002', NULL, 'Focus', 1,
     '{"bpm":[60,110,20],"energy":[0.0,0.45,0.2],"danceability":[0.0,0.5,0.25]}',
     NULL, NULL, 'gapless', 0, 0),

    -- Doing nothing in particular. Unhurried, and not miserable about it.
    ('00000000-0000-4000-9000-000000000003', NULL, 'Chill', 1,
     '{"bpm":[60,105,20],"energy":[0.1,0.5,0.2],"valence":[0.35,0.9,0.25]}',
     NULL, NULL, 'crossfade', 0, 0),

    -- People in a room. Danceable, bright, and loud enough to talk over.
    ('00000000-0000-4000-9000-000000000004', NULL, 'Party', 1,
     '{"bpm":[110,140,15],"energy":[0.6,1.0,0.2],"valence":[0.5,1.0,0.25],"danceability":[0.65,1.0,0.2]}',
     NULL, NULL, 'crossfade', 0, 0),

    -- A road. Steady tempo, present energy, nothing that demands attention.
    ('00000000-0000-4000-9000-000000000005', NULL, 'Driving', 1,
     '{"bpm":[100,140,25],"energy":[0.45,0.9,0.25]}',
     NULL, NULL, 'crossfade', 0, 0),

    -- Going under. As slow and as quiet as the library has.
    ('00000000-0000-4000-9000-000000000006', NULL, 'Sleep', 1,
     '{"bpm":[40,80,15],"energy":[0.0,0.25,0.15],"danceability":[0.0,0.35,0.2]}',
     NULL, NULL, 'crossfade', 0, 0),

    -- Playing. Driving and continuous, and it may be dramatic.
    ('00000000-0000-4000-9000-000000000007', NULL, 'Gaming', 1,
     '{"bpm":[110,170,25],"energy":[0.5,0.95,0.25]}',
     NULL, NULL, 'gapless', 0, 0),

    -- Waking up. Moderate, bright, and on the cheerful side.
    ('00000000-0000-4000-9000-000000000008', NULL, 'Morning', 1,
     '{"bpm":[85,125,20],"energy":[0.3,0.75,0.25],"valence":[0.55,1.0,0.3]}',
     NULL, NULL, 'crossfade', 0, 0);
"#;
