//! Check and Redump in the sidebar: what Redump's step did, in a sentence
//! for its header; and for Check and fix, the facts its sidebar is made
//! of: the comparison it keeps when Redump disagrees with a copy, what's
//! still wrong and why in a line each, and Redump's copy beside this one.

use crate::theme::{font, state_icon, tnum, Pal};
use drt_engine::{StepState, StepView};
use drt_steps::real::mismatch::Mismatch;
use egui::{pos2, vec2, Align2, RichText, Sense, Stroke, Ui};

/// The comparison Check keeps in its record, if it held this dump.
pub fn mismatch(v: &StepView) -> Option<Mismatch> {
    serde_json::from_value(v.record.report.clone()?).ok()
}

fn not_started(v: &StepView) -> bool {
    v.record.started.is_none() && matches!(v.state, StepState::Idle | StepState::Wait)
}

/// Check's header sentence, before it's run: what it's for. Once it has
/// run, its headline says what came of it, so there's none.
pub fn check_say(v: &StepView) -> String {
    if not_started(v) {
        return "Compares the copy's checksums with Redump's list of known-good copies. A match names the disc.".into();
    }
    String::new()
}

/// Redump's header sentence: what it does, or what it did and why.
pub fn redump_say(v: &StepView) -> String {
    if v.state == StepState::Wait && !v.detail.is_empty() {
        return format!("{}, which needs you to decide about the dump. If Check and fix can't name the game, Redump looks it up by its serial.", v.status);
    }
    if not_started(v) {
        return "Looks the game up by its serial when Check and fix can't name it by its checksums, and offers what it finds to Choose.".into();
    }
    let n = v.status.strip_prefix("Found ").and_then(|s| s.split(' ').next()?.parse::<usize>().ok());
    match (v.status.as_str(), v.detail.as_str(), n) {
        ("Not needed", drt_steps::real::game::CHECK_FOUND, _) => "Check and fix found the game by its checksums, so there was nothing to look up. Its match goes on to Choose.".into(),
        ("Not needed", _, _) => "Nothing came from Check and fix, so there was nothing to look up.".into(),
        (s, d, Some(1)) => format!("{s} {d}. It doesn't have this copy's checksums, so Choose asks you whether it's the one."),
        (s, d, Some(_)) => format!("{s} {d}. None has this copy's checksums, so Choose asks you which it is."),
        ("Not in Redump", d, _) => format!("Redump's list has {d}. Choose asks you to name it."),
        ("Nothing to look up", _, _) => "The disc has no serial to look up.".into(),
        ("Couldn't look up", _, _) => "Couldn't reach Redump. Choose asks you to name it.".into(),
        _ => String::new(),
    }
}

/// 230571 as *230,571*.
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A checksum shortened: *43cfec26…*.
fn short(sha1: &str) -> String {
    if sha1.len() > 8 {
        format!("{}…", &sha1[..8])
    } else {
        sha1.to_owned()
    }
}

/// One row of the comparison: what it is, Redump's value, this copy's,
/// and whether they're the same.
fn row(ui: &mut Ui, p: &Pal, what: &str, theirs: &str, ours: &str, same: Option<bool>) {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
    let label_w = (w * 0.24).clamp(70.0, 120.0);
    let col = (w - label_w - 24.0) / 2.0;
    let y = r.center().y;
    let painter = ui.painter();
    painter.text(pos2(r.left(), y), Align2::LEFT_CENTER, what, font(12.5, 500), p.subtle);
    painter.text(pos2(r.left() + label_w, y), Align2::LEFT_CENTER, theirs, tnum(13.0, 400), p.ink);
    painter.text(pos2(r.left() + label_w + col, y), Align2::LEFT_CENTER, ours, tnum(13.0, 400), p.ink);
    if let Some(same) = same {
        let state = if same { StepState::Ok } else { StepState::Bad };
        for s in state_icon(p, state, pos2(r.right() - 10.0, y), 0.85, 0.0) {
            painter.add(s);
        }
    }
    painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line_soft));
    let verdict = match same {
        Some(true) => ", the same",
        Some(false) => ", different",
        None => "",
    };
    let said = format!("{what}: Redump {theirs}, this copy {ours}{verdict}");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Row, true, &said));
}

/// Column heads over the rows.
fn heads(ui: &mut Ui, p: &Pal, first: &str) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    let label_w = (w * 0.24).clamp(70.0, 120.0);
    let col = (w - label_w - 24.0) / 2.0;
    let painter = ui.painter();
    painter.text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, first, font(12.5, 600), p.ink);
    painter.text(pos2(r.left() + label_w, r.center().y), Align2::LEFT_CENTER, "Redump's copy", font(12.5, 500), p.muted);
    painter.text(pos2(r.left() + label_w + col, r.center().y), Align2::LEFT_CENTER, "This copy", font(12.5, 500), p.muted);
    painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line));
}

/// 565 sectors, 1 sector.
fn sectors(n: u64) -> String {
    if n == 1 {
        "1 sector".into()
    } else {
        format!("{} sectors", thousands(n))
    }
}

/// How big the copy is, its files as Check last hashed them (the cue
/// sheet aside), in bytes.
pub fn copy_bytes(m: &Mismatch) -> Option<u64> {
    let n: u64 = m.files.iter().filter_map(|f| f.got.as_ref()).filter(|f| !f.name.to_ascii_lowercase().ends_with(".cue")).map(|f| f.size).sum();
    (n > 0).then_some(n)
}

/// What's still wrong with a copy Check holds, in a line under its
/// headline: *3 sectors still not put right, after 4 rips*; before
/// anything's been tried, what the comparison found.
pub fn still_line(m: &Mismatch) -> String {
    let rips = m.combined.as_ref().map(|c| c.rips.len());
    let after = rips.map(|n| format!(", after {n} rips")).unwrap_or_default();
    let left = match (&m.repaired, &m.combined, &m.fixed) {
        (Some(r), _, _) => Some(r.left()),
        (None, Some(c), _) => Some(c.still_bad),
        (None, None, Some(f)) if f.other_disc => return "Nothing changed: is this the same disc?".into(),
        (None, None, Some(f)) => Some(f.left),
        (None, None, None) => None,
    };
    match left {
        Some(0) if m.in_redump() => "Every sector passes its own check, but it isn't Redump's".into(),
        Some(0) => "Every sector passes its own check".into(),
        Some(n) => format!("{} still not put right{after}", sectors(n)),
        None => {
            let bad = m.read.as_ref().map_or(0, |r| r.bad().max(r.wrong()));
            match (m.in_redump(), m.same_size(), bad) {
                (true, false, _) => "A different size from Redump's copy".into(),
                (_, _, 0) if m.in_redump() => "The same size as Redump's copy, but not the same".into(),
                (_, _, 0) => String::new(),
                (_, _, n) => format!("{} read wrong", sectors(n)),
            }
        }
    }
}

/// Why it needs you, when there's something to say beyond the headline,
/// in a line: what went wrong trying (`note`, the item's), or what the
/// comparison says is the likely cause.
pub fn reason(m: &Mismatch, note: &str) -> Option<String> {
    let n = note.trim();
    if n.starts_with("Couldn't ") || n.contains("SHA-1 is Redump's") {
        return Some(n.to_owned());
    }
    let left = match (&m.repaired, &m.combined, &m.fixed) {
        (Some(r), _, _) => r.left(),
        (None, Some(c), _) => c.still_bad,
        (None, None, Some(f)) => f.left,
        (None, None, None) => m.read.as_ref().map_or(0, |r| r.bad().max(r.wrong())),
    };
    if m.fixed.as_ref().is_some_and(|f| f.other_disc) {
        return Some("Sectors that read right the first time came back different, so the disc in the drive may not be this one.".into());
    }
    if m.in_redump() && !m.same_size() {
        return Some("Its size differs from Redump's copy, so it's likely a different version of the disc.".into());
    }
    let generic = m.read.as_ref().is_some_and(|r| r.generic);
    let tried = m.fixed.is_some() || m.combined.is_some();
    match (left, generic, tried) {
        (0, _, _) if !m.in_redump() => Some("Redump has no copy of this disc to compare with, so it can't be called verified.".into()),
        (0, _, _) => None,
        (n, true, true) => Some(format!(
            "The drive was read with general settings: redumper doesn't know it. Another drive may read {}.",
            if n == 1 { "this one".to_owned() } else { format!("these {}", thousands(n)) }
        )),
        (_, _, true) => Some("Cleaning the disc, or another drive, may help.".into()),
        (_, _, false) if m.read.as_ref().is_some_and(|r| !r.drive_reported()) => Some("The drive didn't report them as bad, so only reading each sector again can find them.".into()),
        _ => None,
    }
}

/// The names of Redump's other copies of the serial, when they differ
/// from the one compared with in more than their revision.
fn others_that_differ(m: &Mismatch) -> Vec<String> {
    let base = |n: &str| n.split(" (Rev").next().unwrap_or(n).trim().to_owned();
    m.others.iter().filter(|o| base(o) != base(&m.entry)).cloned().collect()
}

/// Redump's copy beside this one, file by file: the size and SHA-1 of
/// each, and whether they're the same. Under the copy compared with's name.
pub fn proof(ui: &mut Ui, p: &Pal, m: &Mismatch) {
    ui.label(RichText::new(&m.entry).font(font(13.0, 500)).color(p.ink));
    let others = others_that_differ(m);
    if !others.is_empty() {
        ui.label(RichText::new(format!("Also under {}: {}", m.serial, others.join("; "))).font(font(12.5, 400)).color(p.muted));
    }
    ui.add_space(4.0);
    let many = m.files.len() > 1;
    for (i, f) in m.files.iter().enumerate() {
        let name = f.expected.as_ref().or(f.got.as_ref()).map(|f| f.name.to_ascii_lowercase()).unwrap_or_default();
        let first = if name.ends_with(".iso") {
            "Image".to_owned()
        } else if many {
            format!("Track {}", i + 1)
        } else {
            String::new()
        };
        heads(ui, p, &first);
        let size = |x: &Option<drt_steps::real::mismatch::File>| match x {
            Some(f) => f.sectors().map(|n| format!("{} sectors", thousands(n))).unwrap_or_else(|| format!("{} bytes", thousands(f.size))),
            None => "none".to_owned(),
        };
        let sum = |x: &Option<drt_steps::real::mismatch::File>| x.as_ref().map(|f| short(&f.sha1)).unwrap_or_else(|| "none".into());
        row(ui, p, "Size", &size(&f.expected), &size(&f.got), Some(f.size_ok()));
        row(ui, p, "SHA-1", &sum(&f.expected), &sum(&f.got), Some(f.sha1_ok()));
        ui.add_space(8.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_checksums_read_short() {
        assert_eq!(thousands(230_571), "230,571");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(short("43cfec26aaaabbbb"), "43cfec26…");
    }
}
