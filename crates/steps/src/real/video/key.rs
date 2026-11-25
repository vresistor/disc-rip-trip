//! MakeMKV's key. It won't run without one, even for DVDs, and the free
//! beta key expires about every two months, so left alone the app would
//! break itself every couple of months.
//!
//! So, if you turn it on (it's off until you do), the key is refreshed
//! only when MakeMKV says it isn't registered:
//! fetch the current beta key from MakeMKV's own forum, write it, try
//! again. No network in the usual case, and a refresh only when it's known
//! to be the fix. Lifted from the old app's `key.rs`, with its rules:
//! never touch a purchased key, never write anything that doesn't look
//! like a key, never drop a line of MakeMKV's settings we didn't put there.

use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

/// The thread the developer posts the beta key to: the vendor's own page,
/// not a mirror, since what's read here is written into MakeMKV's config.
const BETA_KEY_THREAD: &str = "https://forum.makemkv.com/forum/viewtopic.php?f=5&t=1053";

/// What a run says when MakeMKV wants a key and you haven't let the app
/// fetch one.
pub const TURNED_OFF: &str =
    "MakeMKV says it isn't registered: its free beta key has probably run out. Enter a key in MakeMKV, or turn on “Keep MakeMKV's beta key up to date” in Disc Rip Trip's MakeMKV settings.";

/// Beta keys start with this; purchased ones don't, which is the only way
/// to tell them apart.
const BETA_PREFIX: &str = "T-";

/// MakeMKV's settings file. `None` where it isn't a file we can see
/// (Windows keeps the key in the registry), which means *don't touch*.
/// `DRT_MAKEMKV_SETTINGS` points it elsewhere, for tests.
pub fn settings_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DRT_MAKEMKV_SETTINGS") {
        return Some(PathBuf::from(p));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "windows") {
        None
    } else if cfg!(target_os = "macos") {
        Some(home.join("Library/MakeMKV/settings.conf"))
    } else {
        // MakeMKV's flatpak keeps it here too: it's given ~/.MakeMKV.
        Some(home.join(".MakeMKV/settings.conf"))
    }
}

/// Shaped like a beta key: length and characters only. A guard against
/// writing an error page into MakeMKV's config, not a validation.
pub fn looks_like_key(s: &str) -> bool {
    (40..=80).contains(&s.len()) && s.starts_with(BETA_PREFIX) && s[2..].chars().all(|c| c.is_ascii_alphanumeric() || "@_%+=/-".contains(c))
}

/// `app_Key` out of MakeMKV's settings.
pub fn parse_key(settings: &str) -> Option<String> {
    settings.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == "app_Key").then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// Whether a purchased key is in place, which must be left alone. Where the
/// settings can't be seen, yes: the other answer is how a paid key gets
/// overwritten.
pub fn has_permanent_key(path: Option<&Path>) -> bool {
    let Some(path) = path else { return true };
    std::fs::read_to_string(path).ok().and_then(|t| parse_key(&t)).is_some_and(|k| !k.is_empty() && !k.starts_with(BETA_PREFIX))
}

/// The settings with `app_Key` set, every other line kept.
pub fn with_key(existing: &str, key: &str) -> String {
    let line = format!("app_Key = \"{key}\"");
    let mut replaced = false;
    let mut out: Vec<String> = existing
        .lines()
        .map(|l| {
            if l.split_once('=').is_some_and(|(n, _)| n.trim() == "app_Key") {
                replaced = true;
                line.clone()
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !replaced {
        out.push(line);
    }
    out.join("\n") + "\n"
}

/// The first thing shaped like a beta key in a page.
pub fn find_key(text: &str) -> Option<String> {
    text.match_indices(BETA_PREFIX).find_map(|(i, _)| {
        let end = text[i..].find(|c: char| !(c.is_ascii_alphanumeric() || "@_%+=/-".contains(c))).map_or(text.len(), |n| i + n);
        let k = &text[i..end];
        looks_like_key(k).then(|| k.to_owned())
    })
}

/// [`refresh`], only if you've turned key updates on; else nothing is
/// fetched or written.
pub async fn refresh_if_allowed(allowed: bool) -> Result<String> {
    if !allowed {
        bail!("{TURNED_OFF}");
    }
    refresh().await
}

/// Fetches the current beta key and installs it. Refuses when a purchased
/// key is there, and when MakeMKV already has the current one (then a
/// stale key isn't the problem).
pub async fn refresh() -> Result<String> {
    let path = settings_path();
    if has_permanent_key(path.as_deref()) {
        bail!("a permanent MakeMKV key is installed; it's left alone");
    }
    let path = path.ok_or_else(|| anyhow!("MakeMKV's settings aren't reachable from here"))?;
    let r = super::super::net::client(std::time::Duration::from_secs(20))?
        .get(BETA_KEY_THREAD)
        // The forum serves nothing to clients that don't say who they are.
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64) disc-rip-trip")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| anyhow!("couldn't reach MakeMKV's forum: {}", e.without_url()))?;
    let body = super::super::net::text(r, super::super::net::PAGE_MAX).await?;
    let key = find_key(&body).ok_or_else(|| anyhow!("no key on MakeMKV's forum page; its layout may have changed"))?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if parse_key(&existing).as_deref() == Some(key.as_str()) {
        bail!("MakeMKV already has the current beta key, so a stale key isn't the problem");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, with_key(&existing, &key))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real key from the forum thread, August 2026.
    const SAMPLE: &str = "T-vwvnrmF87JuRPLYYO8HnIrZR_LCAF7JXs9iJEm10moGpjEgkvmPTsf6Ox4BI@N5rPp";

    #[test]
    fn only_beta_keys_look_like_keys() {
        assert!(looks_like_key(SAMPLE));
        assert!(!looks_like_key("T-short"));
        assert!(!looks_like_key("<html><body>Forbidden</body></html>"));
        assert!(!looks_like_key("M9aB7cD2eF4gH6iJ8kL0mN1oP3qR5sT7uV9wX2yZ4a"), "a purchased key is never ours to replace");
    }

    #[test]
    fn a_key_is_found_in_forum_markup_and_written_keeping_every_other_line() {
        assert_eq!(find_key(&format!(r#"<div class="codebox"><code>{SAMPLE}</code></div>"#)).as_deref(), Some(SAMPLE));
        assert_eq!(find_key("<html>The topic you requested does not exist</html>"), None);
        let conf = "app_SiteInfoString = \"abc\"\napp_Key = \"T-old\"\napp_UpdateLastCheck = \"1\"";
        assert_eq!(parse_key(conf).as_deref(), Some("T-old"));
        let new = with_key(conf, SAMPLE);
        assert_eq!(parse_key(&new).as_deref(), Some(SAMPLE));
        assert!(new.contains("app_SiteInfoString = \"abc\"") && new.contains("app_UpdateLastCheck"));
        assert_eq!(parse_key(&with_key("", SAMPLE)).as_deref(), Some(SAMPLE));
    }

    /// Off, as it starts: MakeMKV's settings aren't read, fetched for or
    /// written, and the run says what to do.
    #[tokio::test]
    async fn key_updates_are_off_until_turned_on_and_then_nothing_is_touched() {
        assert!(!super::super::makemkv::key_updates(), "off by default");
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.conf");
        std::fs::write(&p, "app_Key = \"T-old\"\n").unwrap();
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        let e = refresh_if_allowed(false).await.unwrap_err().to_string();
        assert!(e.contains("Keep MakeMKV's beta key up to date"), "{e}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "app_Key = \"T-old\"\n");
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn a_purchased_key_or_unseen_settings_are_left_alone() {
        let dir = std::env::temp_dir().join(format!("drt-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.conf");
        std::fs::write(&p, "app_Key = \"M-paid-for\"\n").unwrap();
        assert!(has_permanent_key(Some(&p)));
        std::fs::write(&p, "app_Key = \"T-beta\"\n").unwrap();
        assert!(!has_permanent_key(Some(&p)));
        assert!(!has_permanent_key(Some(&dir.join("absent.conf"))), "no settings yet: a beta key may be written");
        assert!(has_permanent_key(None), "settings we can't see: hands off");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
