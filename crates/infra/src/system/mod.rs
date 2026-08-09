//! Operating system services: the clock, data paths, thread priority.

pub mod clock;
pub mod paths;

pub use clock::SystemClock;
pub use paths::AppPaths;
