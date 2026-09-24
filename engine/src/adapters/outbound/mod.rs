pub mod frame_writer;
#[cfg(target_os = "linux")]
pub mod inotify_watcher;
pub mod std_fs;
/// Not `#[cfg(target_os = "linux")]`. It lived inside `inotify_watcher` and so did not exist on
/// any other platform, which nothing noticed because nothing else needed a clock.
pub mod system_clock;
pub mod watch_thread;
pub mod watchers;
