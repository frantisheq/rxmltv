//! Small files the terminal UI keeps next to the settings file: the favorite shows and the channel order.
//! Both are plain text, one entry per line, and are written as soon as something changes.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::groups::normalize;
use crate::util::with_suffix;

/// Reads the non-empty lines of a file; a missing file is simply empty.
pub fn read_lines(path: &Path) -> Vec<String> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .trim_start_matches('\u{feff}')
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Written under a temporary name first, so a crash never leaves half a file.
pub fn write_lines(path: &Path, lines: &[String]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let mut text = lines.join("\n");
    text.push('\n');
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// What two titles are compared by: lower-case, without diacritics.
pub fn title_key(title: &str) -> String {
    normalize(title)
}

pub struct Favorites {
    titles: Vec<String>,
    keys: HashSet<String>,
    path: PathBuf,
}

impl Favorites {
    pub fn load(path: PathBuf) -> Favorites {
        let mut f = Favorites { titles: Vec::new(), keys: HashSet::new(), path: path.clone() };
        for title in read_lines(&path) {
            let key = title_key(&title);
            if !key.is_empty() && f.keys.insert(key) {
                f.titles.push(title);
            }
        }
        f
    }

    pub fn titles(&self) -> &[String] {
        &self.titles
    }

    pub fn contains(&self, title: &str) -> bool {
        self.keys.contains(&title_key(title))
    }

    fn save(&self) -> Result<(), String> {
        write_lines(&self.path, &self.titles).map_err(|e| format!("Could not save {}: {e}", self.path.display()))
    }

    /// Adds the title, or removes it when it is already a favorite. Ok(true): it was added.
    pub fn toggle(&mut self, title: &str) -> Result<bool, String> {
        let key = title_key(title);
        if let Some(pos) = self.titles.iter().position(|t| title_key(t) == key) {
            self.titles.remove(pos);
            self.keys.remove(&key);
            self.save()?;
            return Ok(false);
        }
        self.add(title).map(|_| true)
    }

    pub fn add(&mut self, title: &str) -> Result<(), String> {
        let title = title.trim();
        let key = title_key(title);
        if key.is_empty() {
            return Err("A favorite needs a title.".to_string());
        }
        if self.keys.insert(key) {
            self.titles.push(title.to_string());
            self.save()?;
        }
        Ok(())
    }

    pub fn remove_at(&mut self, index: usize) -> Result<(), String> {
        if index < self.titles.len() {
            let title = self.titles.remove(index);
            self.keys.remove(&title_key(&title));
            self.save()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rxmltv-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn favorites_are_matched_ignoring_case_and_diacritics() {
        let path = temp("fav-match");
        let mut f = Favorites::load(path.clone());
        assert_eq!(f.toggle("Večer s hvězdami").unwrap(), true);
        assert!(f.contains("VECER S HVEZDAMI"));
        assert!(!f.contains("Zprávy"));
        assert_eq!(f.toggle("vecer s hvezdami").unwrap(), false);
        assert!(f.titles().is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn favorites_are_saved_and_loaded() {
        let path = temp("fav-save");
        let mut f = Favorites::load(path.clone());
        f.add("Nautilus").unwrap();
        f.add("Nautilus").unwrap(); // twice: still one
        f.add("Událost").unwrap();
        assert!(f.add("   ").is_err());

        let again = Favorites::load(path.clone());
        assert_eq!(again.titles(), &["Nautilus".to_string(), "Událost".to_string()]);

        let mut again = again;
        again.remove_at(0).unwrap();
        assert_eq!(Favorites::load(path.clone()).titles(), &["Událost".to_string()]);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_is_empty() {
        assert!(read_lines(&temp("does-not-exist")).is_empty());
    }
}
