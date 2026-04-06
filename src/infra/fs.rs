use std::path::Path;

use anyhow::{Context, Result};

pub fn read_to_string(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

#[allow(dead_code)]
pub fn write_string(path: impl AsRef<Path>, contents: &str) -> Result<()> {
    let path = path.as_ref();
    std::fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub fn list_dir(path: impl AsRef<Path>) -> Result<Vec<String>> {
    let path = path.as_ref();
    let mut entries: Vec<String> = std::fs::read_dir(path)
        .with_context(|| format!("failed to read directory {}", path.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let mut name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_dir() {
                name.push('/');
            }
            Some(name)
        })
        .collect();
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use std::fs;

    use super::{list_dir, read_to_string, write_string};

    #[test]
    fn reads_and_writes_utf8_text() {
        let temp = NamedTempFile::new().unwrap();
        write_string(temp.path(), "{\"ok\":true}").unwrap();
        assert_eq!(read_to_string(temp.path()).unwrap(), "{\"ok\":true}");
    }

    #[test]
    fn lists_directory_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.json"), "{}").unwrap();
        fs::create_dir(temp.path().join("dir")).unwrap();
        let entries = list_dir(temp.path()).unwrap();
        assert!(entries.iter().any(|entry| entry == "a.json"));
        assert!(entries.iter().any(|entry| entry == "dir/"));
    }
}
