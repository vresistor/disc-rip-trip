//! Finding a drive's read offset without asking: AccurateRip's list of
//! drives, then what the old app recorded, then measuring with the disc in
//! the drive.
//!
//! AccurateRip publishes every drive's offset as submitted by its users,
//! with how many agree. Both drives on this machine are there exactly:
//! "Panasonic - BD-MLT UJ240AS" +103 (528 agree, 100%), and "ATAPI -
//! eHAU324 A" +6 (108, 100%). The list names some makers differently from
//! the drives themselves, which it says at the top: Matshita is listed as
//! Panasonic, HL-DT-ST as LG Electronics, JLMS as Lite-ON.

use super::tool;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LIST: &str = "https://www.accuraterip.com/driveoffsets.htm";

/// What the list says about one drive.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub offset: i64,
    /// How many people submitted it.
    pub submitted: u32,
    /// What share of them agree, 0–100.
    pub agree: u32,
}

/// The list's rows: (maker, model, offset, submitted, agree). Drives the
/// list marks as purged (no constant offset) are left out.
pub fn parse_list(html: &str) -> Vec<(String, String, Listed)> {
    let mut out = Vec::new();
    for row in html.split("<tr").skip(1) {
        let cells: Vec<String> = row
            .split("<td")
            .skip(1)
            .map(|c| {
                let inner = c.split_once('>').map_or("", |x| x.1);
                let inner = inner.split("</td").next().unwrap_or("");
                strip_tags(inner)
            })
            .collect();
        if cells.len() < 4 {
            continue;
        }
        let Some((maker, model)) = cells[0].split_once(" - ") else { continue };
        let Ok(offset) = cells[1].trim_start_matches('+').parse::<i64>() else { continue };
        out.push((words(maker), words(model), Listed { offset, submitted: cells[2].parse().unwrap_or(0), agree: cells[3].trim_end_matches('%').parse().unwrap_or(0) }));
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    words(&out.replace("&nbsp;", " ").replace("&amp;", "&"))
}

fn words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The list's name for a maker.
fn listed_maker(vendor: &str) -> String {
    match vendor.to_uppercase().as_str() {
        "MATSHITA" => "Panasonic".into(),
        "HL-DT-ST" => "LG Electronics".into(),
        "JLMS" => "Lite-ON".into(),
        _ => vendor.into(),
    }
}

/// Looks a drive up by the maker and model it reports (sysfs `vendor` and
/// `model`). Only an exact model match that everyone agrees on counts.
pub fn find(list: &[(String, String, Listed)], vendor: &str, model: &str) -> Option<Listed> {
    let (maker, model) = (listed_maker(vendor.trim()).to_lowercase(), words(model).to_lowercase());
    list.iter().find(|(m, d, l)| m.to_lowercase() == maker && d.to_lowercase() == model && l.agree == 100).map(|(_, _, l)| l.clone())
}

/// The list, from the cache if it's under a month old.
pub async fn list(cache: &Path) -> Result<Vec<(String, String, Listed)>> {
    let file = cache.join("accuraterip-drive-offsets.htm");
    let fresh = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(30 * 24 * 3600));
    if !fresh {
        let r = super::net::client(Duration::from_secs(30))?.get(LIST).send().await?;
        if !r.status().is_success() {
            return Err(anyhow!("AccurateRip said {}", r.status().as_u16()));
        }
        let bytes = super::net::body(r, super::net::PAGE_MAX).await?;
        std::fs::create_dir_all(cache)?;
        super::place::write(&file, &bytes)?;
    }
    // The page is Latin-1.
    let html: String = std::fs::read(&file)?.into_iter().map(char::from).collect();
    Ok(parse_list(&html))
}

/// What the old app recorded for this drive, keyed by cyanrip's name for it
/// ("MATSHITA BD-MLT UJ240AS 1.01 SCSI CD-ROM").
pub fn from_old_app(vendor: &str, model: &str) -> Option<i64> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let file = home.join(".var/app/io.github.vresistor.disc-rip-trip/config/disc-rip-trip/config.json");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let want = words(&format!("{vendor} {model}")).to_lowercase();
    json.get("drive_offsets")?.as_object()?.iter().find_map(|(k, v)| words(k).to_lowercase().starts_with(&want).then(|| v.as_i64()).flatten())
}

/// Measures the offset with the disc in the drive: cyanrip tries every
/// offset against AccurateRip. Needs a disc AccurateRip knows.
pub async fn measure(device: &str, cwd: &Path, stop: impl std::future::Future<Output = ()>) -> Result<(i64, u32)> {
    let mut cmd = tool::command("cyanrip");
    cmd.args(["-d", device, "-f", "-N", "-U"]);
    let (text, _) = tool::run(cmd, cwd, stop, |_| {}).await?;
    // "Drive offset of +103 found (confidence: 7)!"
    let line =
        text.lines().rev().find(|l| l.contains("Drive offset of") && l.contains("found")).ok_or_else(|| anyhow!("{}", tool::last_meaningful_line(&text).unwrap_or("cyanrip found no offset")))?;
    let offset = line.split("Drive offset of").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.trim_start_matches('+').parse().ok());
    let confidence = line.split("confidence").nth(1).and_then(|r| r.trim_start_matches([':', ' ']).split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0);
    offset.map(|o| (o, confidence)).ok_or_else(|| anyhow!("couldn't read cyanrip's offset: {line}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<table><tr><td>CD Drive</td><td>Correction Offset</td><td>Submitted By</td><td>Percentage Agree</td></tr>
<tr><td><font size="2">ATAPI&nbsp;&nbsp;&nbsp; - eHAU324&nbsp;&nbsp; A</font></td><td>+6</td><td>108</td><td>100%</td></tr>
<tr><td>hp       - BD MLT UJ240</td><td>+103</td><td>18</td><td>100%</td></tr>
<tr><td>Panasonic - BD-MLT UJ240AS</td><td>+103</td><td>528</td><td>100%</td></tr>
<tr><td>Somebody - Wobbly 1</td><td>[Purged]</td><td>9</td><td>60%</td></tr></table>"#;

    #[test]
    fn both_drives_on_this_machine_are_found_by_what_they_report() {
        let list = parse_list(PAGE);
        assert_eq!(list.len(), 3, "the purged drive is left out");
        // What sysfs says: vendor MATSHITA, model "BD-MLT UJ240AS".
        assert_eq!(find(&list, "MATSHITA", "BD-MLT UJ240AS"), Some(Listed { offset: 103, submitted: 528, agree: 100 }));
        assert_eq!(find(&list, "ATAPI", "eHAU324   A"), Some(Listed { offset: 6, submitted: 108, agree: 100 }));
        assert_eq!(find(&list, "MATSHITA", "BD-MLT UJ240"), None, "listed under hp, not Panasonic");
    }
}
