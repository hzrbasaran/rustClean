//! Where rustClean keeps its data: scan history and settings.

use std::path::PathBuf;

/// The data directory. `RUSTCLEAN_DATA_DIR` overrides the platform one.
/// Test builds never fall back to the real one: they use a folder under the
/// temp directory, so `cargo test` cannot touch the user's history or
/// settings.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("RUSTCLEAN_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    if cfg!(test) {
        let name = format!("rustclean-test-data-{}", std::process::id());
        return Some(std::env::temp_dir().join(name));
    }
    Some(dirs::data_dir()?.join("rustClean"))
}
