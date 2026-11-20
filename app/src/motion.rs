//! Motion: whether things move at all, and the one helper they move by.
//!
//! GNOME's *Reduce animations* (`org.gnome.desktop.interface
//! enable-animations`), read once at start, turns every slide, glide and
//! cue into an instant change; so does `DRT_REDUCED_MOTION=1`, for tests
//! and CI. Colour still changes; nothing slides, scales or marches, and
//! the spinner stands still. Nothing here loops: a cue asks for frames
//! only until it's done, so an idle window stays idle.

use std::cell::Cell;

// The UI thread's: each window draws on one thread (tests run several
// windows at once, each on its own).
thread_local! {
    static ON: Cell<bool> = const { Cell::new(true) };
}

/// Whether things move just now.
pub fn on() -> bool {
    ON.with(Cell::get)
}

pub fn set(on: bool) {
    ON.with(|c| c.set(on));
}

/// Whether the system asks for less motion: `DRT_REDUCED_MOTION` ("1"
/// reduced, "0" not), else GNOME's setting, else no.
pub fn reduced_by_system() -> bool {
    match std::env::var("DRT_REDUCED_MOTION").as_deref() {
        Ok("1") | Ok("true") => return true,
        Ok("0") | Ok("false") => return false,
        _ => {}
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
}

/// 0 → 1 as `on` turns true (and back), over `secs` with `ease`; at once
/// when motion is off. Call it every frame with the condition.
pub fn tween(ctx: &egui::Context, id: egui::Id, target: bool, secs: f32, ease: fn(f32) -> f32) -> f32 {
    if !on() {
        return if target { 1.0 } else { 0.0 };
    }
    ctx.animate_bool_with_time_and_easing(id, target, secs, ease)
}

/// How far a one-shot cue that began at `t0` has got (0 → 1) after
/// `secs`; asks for another frame until it's done. 1 at once when motion
/// is off.
pub fn since(ctx: &egui::Context, t0: f64, secs: f32) -> f32 {
    if !on() {
        return 1.0;
    }
    let now = ctx.input(|i| i.time);
    let k = ((now - t0) / secs as f64).clamp(0.0, 1.0) as f32;
    if k < 1.0 {
        ctx.request_repaint();
    }
    k
}

pub use egui::emath::easing::{back_out, cubic_in_out, cubic_out};
