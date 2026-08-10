//! Translation between the application layer and the markup.
//!
//! A view model formats and nothing else. It does not decide what may be
//! played, what happens on a click, or what the library contains — those are
//! the application layer's business (PROJECT_MASTER 4.3). It decides that a
//! missing artist reads "Unknown artist" and that five minutes reads "5:00".
//!
//! Keeping that here rather than in Slint is what makes it testable: these are
//! plain functions over plain data, and the tests below need no window.

pub mod library_vm;
pub mod player_vm;
