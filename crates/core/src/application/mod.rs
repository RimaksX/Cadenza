//! The application layer: use cases, commands, events and view state.
//!
//! Sits between the UI and the domain. It orchestrates — loads entities through
//! ports, applies policies, writes results back, publishes events — and holds no
//! business rules of its own; those live in [`crate::domain::policies`].
//!
//! At M1 only the context exists. Commands, events, DTOs, view state and the
//! services arrive with the milestones that need them: profiles and settings in
//! M3, the library in M4, playback in M5, and the view states the UI reads in M6.

pub mod context;

pub use context::AppContext;
