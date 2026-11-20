//! Check and Redump in the sidebar: what each one does, in a sentence for
//! its header, and Check's comparison of a dump Redump disagrees with:
//! Redump's copy beside this one, file by file, what the read reported,
//! and what that likely means.

use crate::theme::{font, state_icon, tnum, Pal};
use drt_engine::{StepState, StepView};
use drt_steps::real::mismatch::{Mismatch, Read};
use egui::{pos2, vec2, Align2, RichText, Sense, Stroke, Ui};

/// The comparison Check keeps in its record, if it held this dump.
pub fn mismatch(v: &StepView) -> Option<Mismatch> {
    serde_json::from_value(v.record.report.clone()?).ok()
}

fn not_started(v: &StepView) -> bool {
    v.record.started.is_none() && matches!(v.state, StepState::Idle | StepState::Wait)
}

/// Check's header sentence: what it compares, and what came of it.
pub fn check_say(v: &StepView) -> String {
    if not_started(v) {
        return "Compares the copy's checksums with Redump's list of known-good copies. A match names the disc.".into();
    }
    let m = mismatch(v);
    match (v.status.as_str(), &m) {
        ("Doesn't match Redump", Some(m)) => match m.copies() {
            1 => format!("Redump has one copy of {}, and this one doesn't match it. Below, the two side by side.", m.serial),
            n => format!("Redump has {n} copies of {}, and this one matches none of them. Below, it's set beside the closest.", m.serial),
        },
        ("Kept anyway", Some(_)) => "Kept, though it doesn't match Redump's copies.".into(),
        ("Matches Redump", _) => format!("Its checksums match Redump's copy of {}.", v.detail),
        ("Verified", _) => "It matches Redump's copy now, after re-reading the bad parts.".into(),
        ("Not in Redump", _) if v.detail.ends_with("not reached") => format!("No copy in the lists it could reach has these checksums. Couldn't reach: {}.", v.detail.trim_end_matches(" not reached")),
        ("Not in Redump", _) => "No copy in Redump's list has these checksums.".into(),
        ("Couldn't check", _) => "Couldn't reach Redump's list to compare with.".into(),
        _ => String::new(),
    }
}

/// Redump's header sentence: what it does, or what it did and why.
pub fn redump_say(v: &StepView) -> String {
    if v.state == StepState::Wait && !v.detail.is_empty() {
        return format!("{}, which needs you to decide about the dump. If Check can't name the game, Redump looks it up by its serial.", v.status);
    }
    if not_started(v) {
        return "Looks the game up by its serial when Check can't name it by its checksums, and offers what it finds to Choose.".into();
    }
    let n = v.status.strip_prefix("Found ").and_then(|s| s.split(' ').next()?.parse::<usize>().ok());
    match (v.status.as_str(), v.detail.as_str(), n) {
        ("Not needed", drt_steps::real::game::CHECK_FOUND, _) => "Check found the game by its checksums, so there was nothing to look up. Its match goes on to Choose.".into(),
        ("Not needed", _, _) => "Nothing came from Check, so there was nothing to look up.".into(),
        (s, d, Some(1)) => format!("{s} {d}. It doesn't have this copy's checksums, so Choose asks you whether it's the one."),
        (s, d, Some(_)) => format!("{s} {d}. None has this copy's checksums, so Choose asks you which it is."),
        ("Not in Redump", d, _) => format!("Redump's list has {d}. Choose asks you to name it."),
        ("Nothing to look up", _, _) => "The disc has no serial to look up.".into(),
        ("Couldn't look up", _, _) => "Couldn't reach Redump. Choose asks you to name it.".into(),
        _ => String::new(),
    }
}

fn heading(ui: &mut Ui, p: &Pal, text: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(text).font(font(12.5, 600)).color(p.muted));
    ui.add_space(6.0);
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

/// A line of the read: what, how many and where, with a mark.
fn fact(ui: &mut Ui, p: &Pal, what: &str, said: &str, ok: Option<bool>) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
        if let Some(ok) = ok {
            for s in state_icon(p, if ok { StepState::Ok } else { StepState::Bad }, r.center(), 0.85, 0.0) {
                ui.painter().add(s);
            }
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(what).font(font(13.0, 500)).color(p.ink));
            if !said.is_empty() {
                ui.label(RichText::new(said).font(font(12.5, 400)).color(p.muted));
            }
        });
    });
    ui.add_space(6.0);
}

fn read(ui: &mut Ui, p: &Pal, r: &Read) {
    heading(ui, p, "What the read reported");
    if r.drive_reported() {
        fact(ui, p, "The drive reported read errors it couldn't fix", &format!("SCSI {}, C2 {} (in samples, 588 to a sector)", thousands(r.scsi), thousands(r.c2)), Some(false));
    } else {
        fact(ui, p, "The drive reported no read errors", "SCSI 0, C2 0", Some(true));
    }
    if r.sectors.is_empty() {
        fact(ui, p, "Every sector passes its own check", "", Some(true));
    }
    for s in &r.sectors {
        let n = if s.count == 1 { "1 sector".to_owned() } else { format!("{} sectors", thousands(s.count)) };
        let place = r.place(s);
        let said = if place.is_empty() { s.kind.code().to_owned() } else { format!("{place} ({})", s.kind.code()) };
        fact(ui, p, &format!("{n}: {}", s.kind.words().to_lowercase()), &said, Some(false));
    }
    if !r.drive.is_empty() || r.generic {
        let drive = if r.drive.is_empty() { "The drive".to_owned() } else { r.drive.clone() };
        let offset = r.read_offset.map(|o| format!(" and a read offset of {o:+}")).unwrap_or_default();
        let said = if r.generic { format!("redumper doesn't know this drive, so it used general settings{offset}") } else { format!("A drive redumper knows, read with its own settings{offset}") };
        fact(ui, p, &drive, &said, None);
    }
}

/// Check's comparison, under its rows.
pub fn show(ui: &mut Ui, p: &Pal, m: &Mismatch) {
    heading(ui, p, "Compared with Redump");
    ui.label(RichText::new(&m.entry).font(font(13.5, 600)).color(p.ink));
    if !m.others.is_empty() {
        ui.label(RichText::new(format!("Also under {}: {}", m.serial, m.others.join("; "))).font(font(12.5, 400)).color(p.muted));
    }
    ui.add_space(6.0);
    let many = m.files.len() > 1;
    for (i, f) in m.files.iter().enumerate() {
        let name = f.expected.as_ref().or(f.got.as_ref()).map(|f| f.name.to_ascii_lowercase()).unwrap_or_default();
        let first = if name.ends_with(".iso") {
            "Image".to_owned()
        } else if many {
            format!("Track {}", i + 1)
        } else {
            "The disc".to_owned()
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
    if let Some(r) = &m.read {
        read(ui, p, r);
    }
    if !m.reading.is_empty() {
        heading(ui, p, "What it likely means");
        for line in &m.reading {
            ui.label(RichText::new(line).font(font(13.5, 400)).color(p.ink));
            ui.add_space(6.0);
        }
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
