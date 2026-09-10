//! The application layer: use cases, commands, events and view state.
//!
//! Sits between the UI and the domain. It orchestrates — loads entities through
//! ports, applies policies, writes results back, publishes events — and holds no
//! business rules of its own; those live in [`crate::domain::policies`].

pub mod context;
pub mod services;
pub mod view_state;

pub use context::AppContext;
pub use services::ProfileService;
pub use view_state::PlayerView;
