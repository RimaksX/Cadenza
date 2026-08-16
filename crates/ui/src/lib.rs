//! Cadenza UI layer. Renders view state and dispatches commands. No business rules here.
//!
//! The layering rule is enforced by the dependency list rather than by
//! discipline: this crate depends on `cadenza-core` and `slint`, and on nothing
//! else in the workspace. `cadenza-infra` is not reachable from here, so no view
//! can open a database, read a file or touch the audio engine even by accident
//! (PROJECT_MASTER 4.2, 4.3).
//!
//! What crosses the boundary is [`UiServices`] in and a window on the screen
//! out. Everything between is: properties down, callbacks up.

// `deny` rather than the `forbid` the other crates use: the code Slint
// generates from the markup carries its own `allow(unsafe_code)` for the
// vtables it builds, and `forbid` cannot be overridden even by machine output.
// Hand-written code in this crate is still refused an `unsafe` block, which is
// what the rule was for (MASTER_ISSUES 29).
#![deny(unsafe_code)]

use std::sync::Arc;

use cadenza_core::Result;
use cadenza_core::application::ProfileService;
use cadenza_core::application::services::{
    EqService, LibraryService, PlaybackService, PlaylistService, QueueService,
};
use cadenza_core::domain::profile::Profile;

mod app;
mod controller;
pub mod view_models;

// Brings in the types generated from `slint/app_window.slint`: the window
// itself and every struct the markup declares.
slint::include_modules!();

/// What the interface is allowed to talk to.
///
/// Application services only. The composition root builds them out of
/// infrastructure and hands them over already wired, which is why this crate
/// never needs to know that SQLite or cpal exist.
pub struct UiServices {
    /// The library listing.
    pub library: Arc<LibraryService>,
    /// Transport control and the player's view state.
    pub playback: Arc<PlaybackService>,
    /// What plays next. Starting a track goes through this rather than through
    /// [`Self::playback`]: choosing a row is choosing a starting point, and the
    /// queue is what makes the rest of the transport mean anything.
    pub queue: Arc<QueueService>,
    /// The lists the listener keeps.
    pub playlists: Arc<PlaylistService>,
    /// The filters, their presets, and what the listener has them set to.
    pub eq: Arc<EqService>,
    /// Profiles, for the settings screen: the theme and the history switch
    /// belong to the listener rather than to the application.
    pub profiles: Arc<ProfileService>,
    /// Who is listening, if anyone is yet.
    ///
    /// The profile itself rather than the service that manages profiles: the
    /// window reads a name and a theme and changes neither. Switching profiles
    /// and editing them belong to the settings screen, and the service comes
    /// back when that does.
    pub profile: Option<Profile>,
}

/// Opens the window and blocks until the listener closes it.
pub fn run(services: UiServices) -> Result<()> {
    app::run(services)
}
