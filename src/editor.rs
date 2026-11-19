//! What is asked of the editor from the work done in the background: the
//! dialogs to pick a file or where to save one. The editor's answers come
//! on the plugin's own thread, so the asking waits off the runtime.

use std::path::PathBuf;
use std::sync::OnceLock;

use ardoise_plugin::Host;

static HOST: OnceLock<Host> = OnceLock::new();

pub fn set_host(host: Host) {
    let _ = HOST.set(host);
}

pub fn host() -> Option<&'static Host> {
    HOST.get()
}

/// A file picked by the user (none: given up, or no editor).
pub async fn pick_file(title: String, extensions: &'static [&'static str]) -> Option<PathBuf> {
    let host = host()?.clone();
    tokio::task::spawn_blocking(move || host.pick_file(title, extensions).ok().flatten())
        .await
        .ok()
        .flatten()
}

/// Where to save a file, chosen by the user.
pub async fn pick_save_path(name: String) -> Option<PathBuf> {
    let host = host()?.clone();
    tokio::task::spawn_blocking(move || host.pick_save_path(name).ok().flatten())
        .await
        .ok()
        .flatten()
}
