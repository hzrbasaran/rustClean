//! Saved choices (language, theme) as `key=value` lines in the data
//! directory. Saving one key keeps the others.

use std::path::PathBuf;

fn file() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("settings"))
}

/// The saved value of `key`, if any.
pub fn get(key: &str) -> Option<String> {
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
    let _ = std::fs::write(file, text);
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
}
