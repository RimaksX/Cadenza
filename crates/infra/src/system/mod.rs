//! Operating system services: the clock, data paths, thread priority.

pub mod alert;
pub mod clock;
pub mod file_log;
pub mod folder_picker;
pub mod paths;
pub mod windows_priority;

pub use clock::SystemClock;
pub use file_log::FileLog;
pub use folder_picker::SystemFolderPicker;
pub use paths::AppPaths;
pub use windows_priority::WindowsPriority;
