//! Colours, fonts and the small stroke icons, after docs/ux/mockups.html.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Shape, Stroke, Vec2};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub ink: Color32,
    pub muted: Color32,
    /// Small text (12–12.5 px) that still reads: 4.5:1 on its ground.
    pub subtle: Color32,
    /// Lines and hover strokes only, never text.
    pub faint: Color32,
    pub line: Color32,
    /// Between rows and sections, softer than `line`.
    pub line_soft: Color32,
    pub action: Color32,
    pub action_ink: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub bad: Color32,
    pub bad_soft: Color32,
    pub busy: Color32,
    pub busy_soft: Color32,
    pub pending: Color32,
    pub c_src: Color32,
    pub c_read: Color32,
    pub c_look: Color32,
    pub c_make: Color32,
    pub c_put: Color32,
    pub c_chk: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Pal {
    pub fn light() -> Pal {
        Pal {
            dark: false,
            bg: hex(0xe9ecef),
            surface: hex(0xf9fafb),
            raised: hex(0xffffff),
            ink: hex(0x18202b),
            muted: hex(0x4c5563),
            subtle: hex(0x6b7480),
            faint: hex(0x8c95a1),
            line: hex(0xd3d9e0),
            line_soft: hex(0xe8ecf0),
            action: hex(0x2348b8),
            action_ink: hex(0xffffff),
            ok: hex(0x17784a),
            ok_soft: hex(0xdcefe4),
            warn: hex(0xa35a10),
            warn_soft: hex(0xf6e6d2),
            bad: hex(0xb02a22),
            bad_soft: hex(0xf5dcd9),
            busy: hex(0x3b6fe0),
            busy_soft: hex(0xe3ebfb),
            pending: hex(0x8a939f),
            c_src: hex(0x6b7a8f),
            c_read: hex(0x3b6fe0),
            c_look: hex(0x8a5cd0),
            c_make: hex(0xc27a1a),
            c_put: hex(0x2f8a63),
            c_chk: hex(0x17784a),
        }
    }
    pub fn dark() -> Pal {
        Pal {
            dark: true,
            bg: hex(0x15191f),
            surface: hex(0x1c2129),
            raised: hex(0x232a33),
            ink: hex(0xe6eaef),
            muted: hex(0xb3bcc7),
            subtle: hex(0x8693a0),
            faint: hex(0x707b88),
            line: hex(0x313a45),
            line_soft: hex(0x2a313b),
            action: hex(0x7d9cf2),
            action_ink: hex(0x0f1520),
            ok: hex(0x5cc48e),
            ok_soft: hex(0x1d3428),
            warn: hex(0xe3a35a),
            warn_soft: hex(0x3a2c1b),
            bad: hex(0xf07a70),
            bad_soft: hex(0x3d2120),
            busy: hex(0x7d9cf2),
            busy_soft: hex(0x1f2a44),
            pending: hex(0x707b88),
            // Step colours of its own, that read on dark (4.9:1 and up).
            c_src: hex(0x8e9cb0),
            c_read: hex(0x6f95f0),
            c_look: hex(0xa983e6),
            c_make: hex(0xdd9a45),
            c_put: hex(0x4fb385),
            c_chk: hex(0x5cc48e),
        }
    }
    pub fn get(dark: bool) -> Pal {
        if dark {
            Pal::dark()
        } else {
            Pal::light()
        }
    }

    /// The colour of a step's kind, for its icon: by what it does.
    pub fn kind(&self, step: &str) -> Color32 {
        self.kind_known(step).unwrap_or_else(|| match step {
            // One this build doesn't know yet: a guess from its name.
            s if s.contains("choose") => self.c_chk,
            s if s.contains("rip") || s.ends_with("-disc") => self.c_read,
            _ => self.c_put,
        })
    }

    /// The colour of a kind this window knows, by its group.
    pub fn kind_known(&self, step: &str) -> Option<Color32> {
        Some(match step {
            // Source: where discs come from, and go.
            "drive" | "next-disc" => self.c_src,
            // The disc and reading it.
            "audio-cd" | "data-disc" | "game-disc" | "video-disc" | "rip" | "game-rip" | "rip-video" | "image" => self.c_read,
            // Looking things up.
            "musicbrainz" | "cd-text" | "discogs" | "gnudb" | "cover-art" | "lyrics" | "redump" | "tmdb" => self.c_look,
            // Deciding and checking.
            "choose" | "game-choose" | "choose-video" | "hold" | "check" => self.c_chk,
            // Making files.
            "flac" | "mp3" | "opus" | "aac" | "vorbis" | "alac" | "wavpack" | "wav" | "replaygain" | "cue" | "lyrics-file" | "artwork-files" | "chd" | "keep" | "libcrypt" | "m3u" | "transcode"
            | "remux" => self.c_make,
            // Putting them somewhere, and telling you.
            "folder" | "library" | "tell" => self.c_put,
            _ => return None,
        })
    }
}

/// `color-mix(in srgb, a p, b)`.
pub fn mix(a: Color32, p: f32, b: Color32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 * p + y as f32 * (1.0 - p)).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Schibsted Grotesk at four weights, each a family of its own (`w400` ..
/// `w700`), with a few symbols it lacks (✓ ✕ ▾) cut from JetBrains Mono.
/// Anything else missing comes from the system's fonts (eframe's provider).
pub fn fonts() -> FontDefinitions {
    let mut d = FontDefinitions::empty();
    d.font_data.insert("symbols".into(), Arc::new(FontData::from_static(include_bytes!("../assets/DrtSymbols.ttf"))));
    for (w, bytes, tnum) in [
        (400, &include_bytes!("../assets/SchibstedGrotesk-400.ttf")[..], &include_bytes!("../assets/SchibstedGrotesk-400-tnum.ttf")[..]),
        (500, &include_bytes!("../assets/SchibstedGrotesk-500.ttf")[..], &include_bytes!("../assets/SchibstedGrotesk-500-tnum.ttf")[..]),
        (600, &include_bytes!("../assets/SchibstedGrotesk-600.ttf")[..], &include_bytes!("../assets/SchibstedGrotesk-600-tnum.ttf")[..]),
        (700, &include_bytes!("../assets/SchibstedGrotesk-700.ttf")[..], &include_bytes!("../assets/SchibstedGrotesk-700-tnum.ttf")[..]),
    ] {
        let name = format!("sg{w}");
        d.font_data.insert(name.clone(), Arc::new(FontData::from_static(bytes)));
        d.families.insert(FontFamily::Name(format!("w{w}").into()), vec![name.clone(), "symbols".into()]);
        // The same face with tabular figures: its digits all one width
        // (the font's own `.tf` glyphs, alone in a small file), the rest
        // from the face itself. For numbers that change in place.
        let tf = format!("sg{w}-tnum");
        d.font_data.insert(tf.clone(), Arc::new(FontData::from_static(tnum)));
        d.families.insert(FontFamily::Name(format!("t{w}").into()), vec![tf, name, "symbols".into()]);
    }
    d.families.insert(FontFamily::Proportional, vec!["sg400".into(), "symbols".into()]);
    // Tool output and anything that lines up in columns: JetBrains Mono,
    // without ligatures (a tool's `->` stays as it printed it).
    d.font_data.insert("mono".into(), Arc::new(FontData::from_static(include_bytes!("../assets/JetBrainsMonoNL-Regular.ttf"))));
    d.families.insert(FontFamily::Monospace, vec!["mono".into(), "sg400".into(), "symbols".into()]);
    d
}

pub fn font(size: f32, weight: u16) -> FontId {
    FontId::new(size, FontFamily::Name(format!("w{weight}").into()))
}

/// The face with tabular figures: for a number that changes in place (a
/// percentage, a time, a count, a size, a CRC), so it never moves what's
/// beside it.
pub fn tnum(size: f32, weight: u16) -> FontId {
    FontId::new(size, FontFamily::Name(format!("t{weight}").into()))
}

/// Monospace, for tool output.
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Applies the palette to egui's own widgets.
pub fn apply(ctx: &egui::Context, p: &Pal) {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.raised;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.surface;
    v.override_text_color = Some(p.ink);
    v.hyperlink_color = p.action;
    // Selected text tinted with the action colour; a focused field's
    // edge 1.5 px of it.
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(p.action.r(), p.action.g(), p.action.b(), 56);
    v.selection.stroke = Stroke::new(1.5, p.action);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.bg_fill = p.raised;
        w.weak_bg_fill = p.raised;
        w.bg_stroke = Stroke::new(1.0, p.line);
        w.fg_stroke = Stroke::new(1.0, p.ink);
        w.corner_radius = 6.into();
    }
    v.widgets.hovered.weak_bg_fill = mix(p.ink, 0.04, p.raised);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.faint);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.action);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, font(14.0, 400));
        s.text_styles.insert(Button, font(14.0, 500));
        s.text_styles.insert(Small, font(12.0, 400));
        s.text_styles.insert(Heading, font(17.0, 700));
        s.spacing.button_padding = egui::vec2(12.0, 5.0);
        s.spacing.interact_size.y = 28.0;
    });
}

/// The state icon in a tile's corner, 16 units square, at `c` (its centre).
pub fn state_icon(p: &Pal, state: drt_engine::StepState, c: Pos2, z: f32, t: f64) -> Vec<Shape> {
    use drt_engine::StepState::*;
    let r = 7.0 * z;
    let white = Color32::WHITE;
    let s = |w: f32, col: Color32| Stroke::new(w * z, col);
    let pt = |x: f32, y: f32| c + Vec2::new((x - 8.0) * z, (y - 8.0) * z);
    match state {
        Busy => {
            // Turning; still, a ¾ arc, when motion is off.
            let (a0, sweep) = if crate::motion::on() { ((t * std::f64::consts::TAU / 1.2) as f32, std::f32::consts::FRAC_PI_2) } else { (0.0, 1.5 * std::f32::consts::PI) };
            let arc: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = a0 + i as f32 / 16.0 * sweep - std::f32::consts::FRAC_PI_2;
                    c + Vec2::angled(a) * 6.0 * z
                })
                .collect();
            vec![Shape::circle_stroke(c, 6.0 * z, s(2.4, p.busy_soft)), Shape::line(arc, s(2.4, p.busy))]
        }
        Ok => vec![Shape::circle_filled(c, r, p.ok), Shape::line(vec![pt(4.8, 8.3), pt(7.0, 10.5), pt(11.2, 5.9)], s(1.8, white))],
        Needs => vec![Shape::circle_filled(c, r, p.warn), Shape::line_segment([pt(8.0, 4.2), pt(8.0, 8.8)], s(1.9, white)), Shape::circle_filled(pt(8.0, 11.4), 1.1 * z, white)],
        // Waiting is grey: orange means you.
        Wait => vec![Shape::circle_stroke(c, 6.2 * z, s(1.7, p.muted)), Shape::line(vec![pt(8.0, 4.8), pt(8.0, 8.0), pt(10.2, 9.4)], s(1.7, p.muted))],
        Hold => vec![
            Shape::rect_filled(egui::Rect::from_min_max(pt(4.0, 3.0), pt(7.0, 13.0)), 1.0 * z, p.warn),
            Shape::rect_filled(egui::Rect::from_min_max(pt(9.0, 3.0), pt(12.0, 13.0)), 1.0 * z, p.warn),
        ],
        Bad => vec![Shape::circle_filled(c, r, p.bad), Shape::line_segment([pt(5.5, 5.5), pt(10.5, 10.5)], s(1.8, white)), Shape::line_segment([pt(10.5, 5.5), pt(5.5, 10.5)], s(1.8, white))],
        Idle => vec![Shape::circle_stroke(c, 6.2 * z, s(1.7, p.pending))],
    }
}

/// A step kind's stroke icon from the mockup (24-unit viewBox), drawn
/// `size` across with its top-left at `at`. Polylines, circles and rects
/// only; enough to read at a glance.
pub fn kind_icon(step: &str, at: Pos2, size: f32, col: Color32) -> Vec<Shape> {
    glyph(step, at, size, col).unwrap_or_else(|| {
        // A step this build doesn't draw yet: a guess from its name, else
        // a plain tile.
        let like = if step.contains("choose") {
            "choose"
        } else if step.contains("rip") {
            "rip"
        } else if step.ends_with("-disc") {
            "audio-cd"
        } else {
            "step"
        };
        glyph(like, at, size, col).unwrap_or_default()
    })
}

/// Whether a step kind has its own icon (not a guess).
pub fn has_icon(step: &str) -> bool {
    glyph(step, Pos2::ZERO, 24.0, Color32::WHITE).is_some()
}

fn glyph(step: &str, at: Pos2, size: f32, col: Color32) -> Option<Vec<Shape>> {
    let k = size / 24.0;
    let p = |x: f32, y: f32| at + Vec2::new(x * k, y * k);
    let st = Stroke::new(1.8 * k, col);
    let l = |pts: &[(f32, f32)]| Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), st);
    let circ = |x: f32, y: f32, r: f32| Shape::circle_stroke(p(x, y), r * k, st);
    let rect = |x: f32, y: f32, w: f32, h: f32, r: f32| Shape::rect_stroke(egui::Rect::from_min_size(p(x, y), Vec2::new(w * k, h * k)), r * k, st, egui::StrokeKind::Middle);
    let tick = |pts: &[(f32, f32)]| Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), st);
    Some(match step {
        "drive" => vec![rect(2.0, 7.0, 20.0, 10.0, 2.0), l(&[(6.0, 12.0), (14.0, 12.0)]), circ(18.0, 12.0, 0.4)],
        // A drive that isn't there: the drive, struck through.
        "drive-off" => vec![rect(2.0, 7.0, 20.0, 10.0, 2.0), l(&[(6.0, 12.0), (11.0, 12.0)]), l(&[(3.0, 3.0), (21.0, 21.0)])],
        // Eject.
        "next-disc" => vec![l(&[(5.0, 14.0), (12.0, 6.0), (19.0, 14.0), (5.0, 14.0)]), l(&[(5.0, 19.0), (19.0, 19.0)])],
        // Discs: a data disc has a square at its hub; a game disc is the pad; video a film frame.
        "data-disc" => vec![circ(12.0, 12.0, 9.5), rect(9.5, 9.5, 5.0, 5.0, 0.5)],
        "game-disc" => return glyph("game", at, size, col),
        "video-disc" => vec![rect(3.0, 4.0, 18.0, 16.0, 2.0), l(&[(7.0, 4.0), (7.0, 20.0)]), l(&[(17.0, 4.0), (17.0, 20.0)]), l(&[(3.0, 12.0), (7.0, 12.0)]), l(&[(17.0, 12.0), (21.0, 12.0)])],
        // Reading: the same arrow down, whatever the disc.
        "game-rip" | "rip-video" => return glyph("rip", at, size, col),
        // An image: a copy of the disc, two sheets.
        "image" => vec![rect(3.0, 7.0, 14.0, 14.0, 2.0), l(&[(7.0, 3.0), (21.0, 3.0), (21.0, 17.0)])],
        // Looked up: the same glass as the other lookups.
        "redump" | "tmdb" => return glyph("musicbrainz", at, size, col),
        "game-choose" | "choose-video" => return glyph("choose", at, size, col),
        // Checked: a shield with a tick.
        "check" => vec![l(&[(12.0, 2.5), (20.0, 5.5), (20.0, 11.0), (12.0, 21.5), (4.0, 11.0), (4.0, 5.5), (12.0, 2.5)]), tick(&[(8.5, 11.5), (11.0, 14.0), (15.5, 9.0)])],
        // Squeezed into one file.
        "chd" => return glyph("box", at, size, col),
        // Kept as is: a file.
        "keep" => vec![l(&[(14.0, 2.0), (6.0, 2.0), (6.0, 22.0), (19.0, 22.0), (19.0, 7.0), (14.0, 2.0), (14.0, 7.0), (19.0, 7.0)])],
        // A lock.
        "libcrypt" => vec![rect(5.0, 11.0, 14.0, 10.0, 2.0), l(&[(8.0, 11.0), (8.0, 7.5), (10.0, 4.5), (14.0, 4.5), (16.0, 7.5), (16.0, 11.0)])],
        // A playlist.
        "m3u" => vec![l(&[(3.0, 6.0), (15.0, 6.0)]), l(&[(3.0, 12.0), (15.0, 12.0)]), l(&[(3.0, 18.0), (11.0, 18.0)]), l(&[(16.0, 14.0), (16.0, 21.0), (21.5, 17.5), (16.0, 14.0)])],
        // Converted: arrows both ways.
        "transcode" => vec![l(&[(4.0, 8.0), (19.0, 8.0)]), l(&[(15.0, 4.0), (19.0, 8.0), (15.0, 12.0)]), l(&[(20.0, 16.0), (5.0, 16.0)]), l(&[(9.0, 12.0), (5.0, 16.0), (9.0, 20.0)])],
        // Rewrapped: layers.
        "remux" => vec![l(&[(12.0, 3.0), (21.0, 8.0), (12.0, 13.0), (3.0, 8.0), (12.0, 3.0)]), l(&[(3.0, 12.5), (12.0, 17.5), (21.0, 12.5)]), l(&[(3.0, 17.0), (12.0, 22.0), (21.0, 17.0)])],
        // The fallback's plain tile.
        "step" => vec![rect(4.0, 4.0, 16.0, 16.0, 4.0), circ(12.0, 12.0, 1.2)],
        "audio-cd" => vec![circ(12.0, 12.0, 9.5), circ(12.0, 12.0, 2.5)],
        // Hold: pause, two bars.
        "hold" => vec![rect(6.5, 5.0, 3.5, 14.0, 1.0), rect(14.0, 5.0, 3.5, 14.0, 1.0)],
        "musicbrainz" | "discogs" | "gnudb" => vec![circ(11.0, 11.0, 7.0), l(&[(20.5, 20.5), (16.0, 16.0)])],
        "cd-text" => vec![l(&[(4.0, 7.0), (4.0, 4.0), (20.0, 4.0), (20.0, 7.0)]), l(&[(12.0, 4.0), (12.0, 20.0)]), l(&[(9.0, 20.0), (15.0, 20.0)])],
        "cover-art" => vec![rect(3.0, 3.0, 18.0, 18.0, 2.0), circ(9.0, 9.0, 2.0), l(&[(21.0, 15.0), (16.0, 10.0), (5.0, 21.0)])],
        "artwork-files" => {
            vec![rect(2.0, 7.0, 15.0, 15.0, 2.0), circ(7.0, 12.0, 1.6), l(&[(17.0, 18.0), (13.0, 14.0), (4.0, 22.0)]), l(&[(7.0, 4.0), (8.0, 2.5), (21.5, 2.5), (21.5, 16.0), (20.0, 17.0)])]
        }
        "lyrics" | "lyrics-file" => vec![l(&[(9.0, 18.0), (9.0, 5.0), (21.0, 3.0), (21.0, 16.0)]), circ(6.0, 18.0, 3.0), circ(18.0, 16.0, 3.0)],
        "choose" => vec![l(&[(3.0, 6.0), (14.0, 6.0)]), l(&[(3.0, 12.0), (14.0, 12.0)]), l(&[(3.0, 18.0), (10.0, 18.0)]), l(&[(16.0, 16.0), (18.0, 18.0), (22.0, 14.0)])],
        "rip" => vec![l(&[(12.0, 3.0), (12.0, 14.0)]), l(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]), l(&[(4.0, 17.0), (4.0, 21.0), (20.0, 21.0), (20.0, 17.0)])],
        "flac" | "mp3" | "opus" | "aac" | "vorbis" | "alac" | "wavpack" | "wav" => {
            [(3.0, 10.0, 14.0), (7.0, 7.0, 17.0), (11.0, 4.0, 20.0), (15.0, 8.0, 16.0), (19.0, 10.5, 13.5)].iter().map(|&(x, a, b)| l(&[(x, a), (x, b)])).collect()
        }
        "replaygain" => vec![
            l(&[(4.0, 21.0), (4.0, 3.0)]),
            l(&[(12.0, 21.0), (12.0, 3.0)]),
            l(&[(20.0, 21.0), (20.0, 3.0)]),
            l(&[(2.0, 14.0), (6.0, 14.0)]),
            l(&[(10.0, 8.0), (14.0, 8.0)]),
            l(&[(18.0, 16.0), (22.0, 16.0)]),
        ],
        // Folder (and Library, its old name): one icon, whatever its label.
        "folder" | "library" => vec![rect(4.0, 4.0, 4.0, 16.0, 0.0), rect(10.0, 4.0, 4.0, 16.0, 0.0), l(&[(16.0, 5.5), (19.8, 4.5), (22.8, 20.0), (19.0, 21.0), (16.0, 5.5)])],
        "cue" => vec![l(&[(14.0, 2.0), (6.0, 2.0), (6.0, 22.0), (19.0, 22.0), (19.0, 7.0), (14.0, 2.0), (14.0, 7.0), (19.0, 7.0)]), l(&[(9.0, 13.0), (15.0, 13.0)]), l(&[(9.0, 17.0), (15.0, 17.0)])],
        "tell" => vec![l(&[(6.0, 17.0), (6.0, 8.0), (9.0, 3.5), (15.0, 3.5), (18.0, 8.0), (18.0, 17.0)]), l(&[(3.0, 17.0), (21.0, 17.0)]), l(&[(10.3, 21.0), (13.7, 21.0)])],
        "disc-row" => vec![circ(12.0, 12.0, 9.5), circ(12.0, 12.0, 2.5)],
        "tv" => vec![rect(2.0, 7.0, 20.0, 14.0, 2.0), l(&[(7.0, 2.0), (12.0, 7.0), (17.0, 2.0)])],
        "game" => vec![rect(2.0, 6.0, 20.0, 12.0, 5.0), l(&[(6.5, 12.0), (10.5, 12.0)]), l(&[(8.5, 10.0), (8.5, 14.0)])],
        "box" => vec![rect(2.0, 4.0, 20.0, 5.0, 1.0), l(&[(4.0, 9.0), (4.0, 20.0), (20.0, 20.0), (20.0, 9.0)]), l(&[(10.0, 13.0), (14.0, 13.0)])],
        "folder-link" => vec![l(&[(3.0, 6.75), (9.0, 6.75), (11.25, 9.0), (21.0, 9.0), (21.0, 19.5), (3.0, 19.5), (3.0, 6.75)])],
        "pencil" => vec![l(&[(16.5, 3.5), (19.5, 6.5), (7.0, 19.0), (3.0, 20.0), (4.0, 16.0), (16.5, 3.5)])],
        "stop" => vec![rect(6.0, 6.0, 12.0, 12.0, 1.5)],
        "fit" => {
            vec![l(&[(3.0, 9.0), (3.0, 3.0), (9.0, 3.0)]), l(&[(21.0, 9.0), (21.0, 3.0), (15.0, 3.0)]), l(&[(3.0, 15.0), (3.0, 21.0), (9.0, 21.0)]), l(&[(21.0, 15.0), (21.0, 21.0), (15.0, 21.0)])]
        }
        "collapse" => {
            vec![l(&[(9.0, 3.0), (9.0, 9.0), (3.0, 9.0)]), l(&[(15.0, 3.0), (15.0, 9.0), (21.0, 9.0)]), l(&[(9.0, 21.0), (9.0, 15.0), (3.0, 15.0)]), l(&[(15.0, 21.0), (15.0, 15.0), (21.0, 15.0)])]
        }
        "plus" => vec![l(&[(12.0, 5.0), (12.0, 19.0)]), l(&[(5.0, 12.0), (19.0, 12.0)])],
        "x" => vec![l(&[(6.0, 6.0), (18.0, 18.0)]), l(&[(18.0, 6.0), (6.0, 18.0)])],
        "tick" => vec![Shape::line([(5.0, 12.5), (9.5, 17.0), (19.0, 7.0)].iter().map(|&(x, y)| p(x, y)).collect(), Stroke::new(2.6 * k, col))],
        "play" => vec![l(&[(7.0, 4.0), (7.0, 20.0), (20.0, 12.0), (7.0, 4.0)])],
        "done" => vec![l(&[(5.0, 12.5), (9.5, 17.0), (19.0, 7.0)])],
        "trash" => vec![l(&[(4.0, 6.0), (20.0, 6.0)]), l(&[(9.0, 6.0), (9.0, 3.5), (15.0, 3.5), (15.0, 6.0)]), l(&[(6.0, 6.0), (7.0, 21.0), (17.0, 21.0), (18.0, 6.0)])],
        "undo" => vec![l(&[(9.0, 14.0), (4.0, 9.0), (9.0, 4.0)]), l(&[(4.0, 9.0), (14.0, 9.0), (18.0, 10.5), (20.0, 14.0), (18.0, 17.5), (14.0, 19.0), (10.0, 19.0)])],
        "redo" => vec![l(&[(15.0, 14.0), (20.0, 9.0), (15.0, 4.0)]), l(&[(20.0, 9.0), (10.0, 9.0), (6.0, 10.5), (4.0, 14.0), (6.0, 17.5), (10.0, 19.0), (14.0, 19.0)])],
        "minus" => vec![l(&[(5.0, 12.0), (19.0, 12.0)])],
        _ => return None,
    })
}
