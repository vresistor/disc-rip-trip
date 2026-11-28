//! What the window keeps between runs: the recipes and the settings that
//! belong to drives, as JSON in one folder.

use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};
use trip_engine::{Recipe, Settings};

pub struct Store {
    pub dir: PathBuf,
    /// Set when `recipes.json` was there but didn't read; then it's never
    /// overwritten, so nothing of yours is lost to a bug.
    pub recipes_unreadable: Option<String>,
    /// The same for `settings.json`: the drives' offsets aren't lost to a
    /// save over a file that didn't read.
    pub settings_unreadable: Option<String>,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into(), recipes_unreadable: None, settings_unreadable: None }
    }

    /// `$XDG_CONFIG_HOME/trip`, or `~/.config/trip`.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_else(|| PathBuf::from(".")).join("trip")
    }

    pub fn recipes_path(&self) -> PathBuf {
        self.dir.join("recipes.json")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    /// Copies `recipes.json` and `settings.json` from `from` when this
    /// folder has no recipes yet. Inside the flatpak that's the ones a build
    /// from source kept in `~/.config/trip`. Copied, not moved, so
    /// that build keeps its own. Gives what it copied.
    pub fn adopt(&self, from: &Path) -> Vec<PathBuf> {
        if from == self.dir || self.recipes_path().exists() || !from.join("recipes.json").is_file() {
            return Vec::new();
        }
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            eprintln!("couldn't make {}: {e}", self.dir.display());
            return Vec::new();
        }
        let mut copied = Vec::new();
        for name in ["settings.json", "recipes.json"] {
            let src = from.join(name);
            if src.is_file() {
                match std::fs::read(&src).map_err(anyhow::Error::from).and_then(|b| write(&self.dir.join(name), &b)) {
                    Ok(_) => copied.push(src),
                    Err(e) => eprintln!("couldn't copy {}: {e}", src.display()),
                }
            }
        }
        copied
    }

    /// The recipes, seeded with the built-in ones (and saved) the first time.
    pub fn load_recipes(&mut self, seed: impl FnOnce() -> Vec<Recipe>) -> Vec<Recipe> {
        let path = self.recipes_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(e) => {
                    self.recipes_unreadable = Some(format!("{}: {e}", path.display()));
                    seed()
                }
            },
            Err(_) => {
                let r = seed();
                if let Err(e) = self.save_recipes(&r) {
                    eprintln!("couldn't save the recipes: {e:#}");
                }
                r
            }
        }
    }

    pub fn save_recipes(&self, recipes: &[Recipe]) -> Result<()> {
        if self.recipes_unreadable.is_some() {
            return Ok(());
        }
        write(&self.recipes_path(), serde_json::to_string_pretty(recipes)?.as_bytes())
    }

    pub fn load_settings(&mut self) -> Option<Settings> {
        let text = std::fs::read_to_string(self.settings_path()).ok()?;
        // One an older build left readable by everyone is made yours alone now,
        // not at the next save.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::metadata(self.settings_path()).is_ok_and(|m| m.permissions().mode() & 0o077 != 0) {
                if let Err(e) = std::fs::set_permissions(self.settings_path(), std::fs::Permissions::from_mode(0o600)) {
                    eprintln!("couldn't make {} private: {e}", self.settings_path().display());
                }
            }
        }
        match serde_json::from_str(&text) {
            Ok(s) => Some(s),
            Err(e) => {
                self.settings_unreadable = Some(format!("{}: {e}", self.settings_path().display()));
                None
            }
        }
    }

    /// Light, dark or as the system is: "system", "light" or "dark".
    pub fn load_appearance(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join("appearance")).ok().map(|s| s.trim().to_owned())
    }

    pub fn save_appearance(&self, a: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("appearance"), a)?;
        Ok(())
    }

    /// Whether the canvas follows what's happening: "on" or "off".
    pub fn load_follow(&self) -> Option<bool> {
        match std::fs::read_to_string(self.dir.join("follow")).ok()?.trim() {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }
    }

    pub fn save_follow(&self, on: bool) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("follow"), if on { "on" } else { "off" })?;
        Ok(())
    }

    /// Whether the recipe list stays open: "open", else it folds to a rail
    /// once a recipe is chosen.
    pub fn load_list_pinned(&self) -> bool {
        std::fs::read_to_string(self.dir.join("recipe-list")).is_ok_and(|s| s.trim() == "open")
    }

    pub fn save_list_pinned(&self, on: bool) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("recipe-list"), if on { "open" } else { "folds" })?;
        Ok(())
    }

    /// Which view of a disc's map: "disc" or "sectors".
    pub fn load_map_view(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join("map-view")).ok().map(|s| s.trim().to_owned())
    }

    pub fn save_map_view(&self, v: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("map-view"), v)?;
        Ok(())
    }

    /// Where MakeMKV may be run from, as you chose it; none if you haven't.
    pub fn load_makemkv(&self) -> Option<crate::makemkv::Saved> {
        serde_json::from_str(&std::fs::read_to_string(self.dir.join("makemkv")).ok()?).ok()
    }

    /// Saves it, or (`None`) forgets it.
    pub fn save_makemkv(&self, s: Option<&crate::makemkv::Saved>) -> Result<()> {
        match s {
            Some(s) => write(&self.dir.join("makemkv"), serde_json::to_string_pretty(s)?.as_bytes()),
            None => match std::fs::remove_file(self.dir.join("makemkv")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            },
        }
    }

    /// Whether MakeMKV's beta key is kept up to date: "on", else off.
    pub fn load_makemkv_key_updates(&self) -> bool {
        std::fs::read_to_string(self.dir.join("makemkv-key-updates")).is_ok_and(|s| s.trim() == "on")
    }

    pub fn save_makemkv_key_updates(&self, on: bool) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("makemkv-key-updates"), if on { "on" } else { "off" })?;
        Ok(())
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        if self.settings_unreadable.is_some() {
            return Ok(());
        }
        write(&self.settings_path(), serde_json::to_string_pretty(s)?.as_bytes())
    }
}

/// Writes through a temporary file, so a crash never leaves half a file.
/// Readable by you alone: the settings hold API keys, a Discogs token, an
/// email address and an ntfy topic.
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e).with_context(|| format!("clearing {}", tmp.display())),
        _ => {}
    }
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut f = open.open(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
    f.write_all(bytes).with_context(|| format!("writing {}", tmp.display()))?;
    drop(f);
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// A run kept after it ended: what its tiles showed, what it found, and
/// what its tools printed (the last few hundred lines of each).
#[derive(Clone, Debug, Default)]
pub struct Kept {
    pub snap: trip_engine::Snapshot,
    pub context: trip_engine::Context,
    pub raw: std::collections::BTreeMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_are_adopted_once_and_never_over_ones_already_there() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("old"), t.path().join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("recipes.json"), "[]").unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        let store = Store::new(&new);
        assert_eq!(store.adopt(&old).len(), 2);
        assert_eq!(std::fs::read_to_string(new.join("recipes.json")).unwrap(), "[]");
        assert!(old.join("recipes.json").exists(), "copied, not moved");
        // Already has recipes: left alone.
        std::fs::write(old.join("recipes.json"), "[1]").unwrap();
        assert!(store.adopt(&old).is_empty());
        assert_eq!(std::fs::read_to_string(new.join("recipes.json")).unwrap(), "[]");
        // Nothing to adopt.
        assert!(Store::new(t.path().join("other")).adopt(&t.path().join("none")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_settings_are_readable_by_you_alone() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let t = tempfile::tempdir().unwrap();
        let store = Store::new(t.path().join("new"));
        // Over a file that was readable by everyone, as an older build left it.
        std::fs::create_dir_all(&store.dir).unwrap();
        std::fs::write(store.settings_path(), "{}").unwrap();
        std::fs::set_permissions(store.settings_path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        store.save_settings(&Settings::default()).unwrap();
        assert_eq!(mode(&store.settings_path()), 0o600);
        // Adopted from a build that kept them readable by everyone.
        let old = t.path().join("old");
        std::fs::create_dir_all(&old).unwrap();
        for f in ["recipes.json", "settings.json"] {
            std::fs::write(old.join(f), "{}").unwrap();
            std::fs::set_permissions(old.join(f), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let fresh = Store::new(t.path().join("fresh"));
        fresh.adopt(&old);
        assert_eq!(mode(&fresh.settings_path()), 0o600);
        // Left readable by everyone, then only read.
        let mut store = Store::new(&old);
        store.load_settings();
        assert_eq!(mode(&old.join("settings.json")), 0o600);
    }
}
