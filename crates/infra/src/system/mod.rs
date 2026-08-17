//! Operating system services: the clock, data paths, thread priority.

pub mod clock;
pub mod folder_picker;
pub mod paths;
pub mod windows_priority;

pub use clock::SystemClock;
pub use folder_picker::SystemFolderPicker;
pub use paths::AppPaths;
pub use windows_priority::WindowsPriority;
