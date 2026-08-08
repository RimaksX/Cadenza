//! Cadenza core: domain entities, value objects, policies, ports, application services.
//!
//! This crate must never depend on `cadenza-infra` or `cadenza-ui`
//! (PROJECT_MASTER section 4.2). It performs no IO of its own: everything that
//! touches a disk, a database, an audio device or the clock goes through a port
//! declared in [`domain::ports`] and implemented by the infrastructure layer.

#![forbid(unsafe_code)]

pub mod application;
pub mod domain;
pub mod error;

pub use error::{CoreError, Result};
