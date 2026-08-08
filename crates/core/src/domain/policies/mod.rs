//! Pure decision rules.
//!
//! Every function here is deterministic and side-effect free: no clock, no
//! randomness, no IO. Time and randomness arrive as arguments so that the rules
//! stay testable, which is why these are the most heavily tested files in the
//! crate.
//!
//! Policies whose formulas PROJECT_MASTER fixes are implemented now. Policies
//! whose weights it leaves open — the radio ranking of section 10.4, the smart
//! shuffle selection of section 9.4 — expose their hard constraints here and
//! gain their scoring in M12 and M13.

pub mod duplicate_policy;
pub mod eq_policy;
pub mod history_policy;
pub mod playback_policy;
pub mod radio_policy;
pub mod retention_policy;
pub mod shuffle_policy;
pub mod transition_policy;

/// The score used when a comparison cannot be made because a feature is missing.
///
/// Neutral rather than zero: an unanalysed track should be neither favoured nor
/// blacklisted, and zero would read as "maximally incompatible".
pub const NEUTRAL_SCORE: f32 = 0.5;
