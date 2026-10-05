//! Saved choices (language, theme) as `key=value` lines in the data
//! directory. Saving one key keeps the others.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

fn file() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("settings"))
}

/// Held while the file is read or rewritten, so two saves at once (from
/// different threads) never drop each other's key.
static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    // A thread that panicked while holding it left the file whole.
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The saved value of `key`, if any.
pub fn get(key: &str) -> Option<String> {
    let _held = lock();
    let text = std::fs::read_to_string(file()?).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(str::to_string)
}

/// Saves `key=value`, keeping the other keys. Failures are ignored: a
/// setting that is not remembered is not worth an error.
pub fn set(key: &str, value: &str) {
    let Some(file) = file() else {
        return;
    };
    let _held = lock();
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    let mut text: String = old
        .lines()
        .filter(|l| l.split_once('=').is_none_or(|(k, _)| k != key))
        .map(|l| format!("{l}\n"))
        .collect();
    text.push_str(&format!("{key}={value}\n"));
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written next to it and renamed, so a reader never sees half a file.
    let tmp = file.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(tmp, file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_other_keys() {
        // Test builds use a data directory under the temp directory.
        set("test-a", "1");
        set("test-b", "2");
        set("test-a", "3");
        assert_eq!(get("test-a").as_deref(), Some("3"));
        assert_eq!(get("test-b").as_deref(), Some("2"));
        assert_eq!(get("test-missing"), None);
        // A key that is a prefix of another is not confused with it.
        set("test-ab", "4");
        assert_eq!(get("test-a").as_deref(), Some("3"));
    }

    #[test]
    fn saves_from_many_threads_keep_every_key() {
        // Tests (and the interface's threads) may save at the same time;
        // no save may drop another's key.
        let keys: Vec<String> = (0..16).map(|i| format!("race-{i}")).collect();
        std::thread::scope(|s| {
            for k in &keys {
                s.spawn(move || {
                    for n in 0..20 {
                        set(k, &n.to_string());
                    }
                });
            }
        });
        for k in &keys {
            assert_eq!(get(k).as_deref(), Some("19"), "{k}");
        }
    }
}
