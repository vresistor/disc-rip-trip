//! A step's card: 288 × 192, a 3:2 card on a 16 px module, with the same
//! five slots in the same places on every one (docs/ux/cards.md):
//!
//! 1. the name and the state glyph, 16–32;
//! 2. the settings, label and value, two rows of 16 from 32 (space kept);
//! 3. the outcome, 20 px, ending on the lower third line (128), up to two
//!    lines, with at most one qualifier;
//! 4. one detail line, 136–152;
//! 5. the item strip, 168–176.
//!
//! What goes where, and what is cut, is worked out here at 100 % from the
//! text's measured widths ([`build`]); [`paint`] only scales it, so a card
//! reads the same at every zoom and the tests can ask what was cut.

// Every figure on a card is tabular: its outcome, counts and times change
// in place, and must not move what is beside them.
use crate::theme::{kind_icon, state_icon, tnum as font, Pal};
use drt_engine::{ItemState, StepState, StepView};
use egui::{pos2, vec2, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, StrokeKind};

pub const W: f32 = 288.0;
pub const H: f32 = 192.0;
/// The module, and every margin.
pub const M: f32 = 16.0;
/// The first third line: settings values start here.
pub const THIRD: f32 = 96.0;
/// The lower third line: the outcome sits on it.
pub const OUT_BOTTOM: f32 = 128.0;
pub const OUT_LINE: f32 = 24.0;
pub const NAME: (f32, f32) = (16.0, 32.0);
pub const SETTINGS: [(f32, f32); 2] = [(32.0, 48.0), (48.0, 64.0)];
pub const DETAIL: (f32, f32) = (136.0, 152.0);
pub const STRIP: (f32, f32) = (168.0, 176.0);
/// Cover art's picture: the lower-left cell, 96 × 96.
pub const COVER: Rect = Rect { min: Pos2 { x: 16.0, y: 80.0 }, max: Pos2 { x: 112.0, y: 176.0 } };
/// Where slots 3 to 5 start beside the cover.
pub const BESIDE_COVER: f32 = 128.0;
/// The name starts after the 14 px icon and 8 px.
const NAME_X: f32 = 38.0;
/// The state glyph's centre.
const GLYPH: Pos2 = Pos2 { x: W - M - 7.0, y: 24.0 };
/// A cell narrower than this merges with its neighbours.
const MIN_CELL: f32 = 3.0;

/// Type: three sizes from one ratio of 1.25.
pub const SMALL: f32 = 13.0;
pub const BIG: f32 = 20.0;

/// The state as the card shows it, by its glyph and outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Idle,
    Wait,
    Busy,
    Ok,
    /// Needs you: a question, an item to decide, a hold that's full.
    Needs,
    Bad,
    /// Went on without doing its job (no answer, skipped): nothing failed.
    Skipped,
}

impl Shown {
    /// What a step's view comes to on its card. `retries`: the step offers
    /// ways to settle an item, so one that went wrong is a decision (orange)
    /// rather than a failure (red).
    pub fn of(v: &StepView, retries: bool) -> Shown {
        let all_skipped = !v.items.is_empty() && v.items.iter().all(|i| i.state == ItemState::Skipped);
        match v.state {
            StepState::Idle => Shown::Idle,
            StepState::Wait => Shown::Wait,
            // Started, but only waiting for what's before it to end (Next
            // disc): a clock, not a spinner; nothing to watch.
            StepState::Busy if v.status.is_empty() && v.detail.is_empty() && v.items.is_empty() && v.progress.is_none() => Shown::Wait,
            StepState::Busy => Shown::Busy,
            StepState::Ok if all_skipped || v.status == "Skipped" => Shown::Skipped,
            StepState::Ok => Shown::Ok,
            StepState::Needs => Shown::Needs,
            // Holding: waiting (a clock), until all is in and it waits for you.
            StepState::Hold if v.detail == crate::card::WAITS_FOR_YOU && v.count(ItemState::Waiting) > 0 => Shown::Needs,
            StepState::Hold => Shown::Wait,
            StepState::Bad if retries && v.count(ItemState::Bad) > 0 => Shown::Needs,
            StepState::Bad => Shown::Bad,
        }
    }

    /// The engine's state whose glyph this shows.
    pub fn glyph_state(self) -> StepState {
        match self {
            Shown::Idle => StepState::Idle,
            Shown::Wait => StepState::Wait,
            Shown::Busy => StepState::Busy,
            Shown::Ok => StepState::Ok,
            Shown::Needs => StepState::Needs,
            Shown::Bad | Shown::Skipped => StepState::Bad,
        }
    }

    /// The state a cue plays for: what the card shows (a Rip with a track
    /// to decide asks, it doesn't fail); a skip plays none.
    pub fn cue_state(self) -> StepState {
        match self {
            Shown::Skipped => StepState::Idle,
            s => s.glyph_state(),
        }
    }

    /// The 2 px outline's colour: only working, needing you or failed.
    pub fn outline(self, p: &Pal) -> Option<Color32> {
        match self {
            Shown::Busy => Some(p.busy),
            Shown::Needs => Some(p.warn),
            Shown::Bad => Some(p.bad),
            _ => None,
        }
    }
}

/// Hold's detail once everything is in: it waits for you.
pub const WAITS_FOR_YOU: &str = drt_steps::hold::WAITS_FOR_YOU;

/// One cell of the item strip, by the strip's legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cell {
    // In order of urgency, for cells merged on a long strip.
    Bad,
    Needs,
    Busy,
    Wait,
    Done,
    Verified,
    Out,
}

/// The cells: one per item the step handled, padded with waiting ones for
/// those still to come; a step that handles the disc as a whole has one.
pub fn cells(v: &StepView, shown: Shown) -> Vec<Cell> {
    let whole = match shown {
        Shown::Busy => Cell::Busy,
        Shown::Ok => Cell::Done,
        Shown::Needs => Cell::Needs,
        Shown::Bad => Cell::Bad,
        Shown::Skipped => Cell::Out,
        Shown::Idle | Shown::Wait => Cell::Wait,
    };
    let mut out: Vec<Cell> = v
        .items
        .iter()
        .map(|it| match it.state {
            // Held: arrived and kept, waiting only for you.
            ItemState::Waiting if v.state == StepState::Hold => Cell::Done,
            ItemState::Waiting => Cell::Wait,
            ItemState::Busy if shown == Shown::Bad => Cell::Bad,
            ItemState::Busy => Cell::Busy,
            ItemState::Ok if it.verified => Cell::Verified,
            ItemState::Ok => Cell::Done,
            ItemState::Bad if shown == Shown::Needs => Cell::Needs,
            ItemState::Bad => Cell::Bad,
            ItemState::Skipped => Cell::Out,
        })
        .collect();
    let total = v.total.unwrap_or(0);
    if out.is_empty() {
        return vec![whole; total.max(1)];
    }
    // Still to come, while it can still come: a step that's done handled
    // what it was given (a track left out never reached it).
    if !matches!(shown, Shown::Ok | Shown::Skipped | Shown::Bad) {
        while out.len() < total {
            out.push(Cell::Wait);
        }
    }
    out
}

/// The strip's cells laid out from `x0` to `x1`: equal widths 2 apart;
/// 1 apart when that's too tight; and when even that would make them
/// narrower than 3, neighbours merge, each merged cell taking its most
/// urgent member's state. Gives (left, right, cell).
pub fn strip_layout(cells: &[Cell], x0: f32, x1: f32) -> Vec<(f32, f32, Cell)> {
    let w = x1 - x0;
    let n = cells.len().max(1);
    let fit = |n: usize, gap: f32| (w - gap * (n as f32 - 1.0)) / n as f32;
    let (groups, gap): (Vec<Cell>, f32) = if fit(n, 2.0) >= MIN_CELL {
        (cells.to_vec(), 2.0)
    } else if fit(n, 1.0) >= MIN_CELL {
        (cells.to_vec(), 1.0)
    } else {
        let most = ((w + 1.0) / (MIN_CELL + 1.0)).floor().max(1.0) as usize;
        let per = n.div_ceil(most);
        (cells.chunks(per).map(|c| *c.iter().min().unwrap()).collect(), 1.0)
    };
    let cw = fit(groups.len(), gap);
    groups.iter().enumerate().map(|(k, c)| (x0 + k as f32 * (cw + gap), x0 + k as f32 * (cw + gap) + cw, *c)).collect()
}

/// A setting as the card shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CardSetting {
    pub label: String,
    pub value: String,
    /// Changed by you: a blue dot before the value.
    pub yours: bool,
    /// A folder: shortened keeping both ends.
    pub path: bool,
    /// The setting's help, shown on hovering its label.
    pub help: Option<&'static str>,
}

/// What a card is made from.
pub struct Input<'a> {
    pub v: &'a StepView,
    pub settings: &'a [CardSetting],
    /// Its settings can't change now (ripping): a lock in the label column.
    pub locked: bool,
    /// It has a picture in the lower-left cell (Cover art).
    pub cover: bool,
    /// The question up is this step's.
    pub asking: bool,
    /// The step offers ways to settle an item.
    pub retries: bool,
}

/// Measures a line of text at 100 %: its width at `size` and `weight`.
pub type Measure<'a> = &'a dyn Fn(&str, f32, u16) -> f32;

/// A settings row, fitted.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    /// One line, or two when a single setting's value wraps.
    pub value: Vec<String>,
    pub yours: bool,
    /// A label too long to share its column with the lock: the lock goes
    /// before the value instead, like the dot.
    pub lock_by_value: bool,
    /// The setting's help, shown on hovering the label.
    pub help: Option<&'static str>,
}

/// A card, fitted: what each slot says, at 100 %.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub shown: Shown,
    pub name: String,
    pub rows: Vec<Row>,
    pub locked: bool,
    /// The outcome's lines, the last on the lower third line.
    pub outcome: Vec<String>,
    pub qualifier: String,
    pub detail: String,
    /// The detail is what it's on right now (ink), not a fact (secondary).
    pub working: bool,
    pub cells: Vec<Cell>,
    /// Not run yet: drawn as a blueprint (dashed, see-through).
    pub plan: bool,
    pub cover: bool,
    /// The full text of everything that had to be shortened.
    pub cut: Vec<String>,
}

impl Card {
    /// Where slots 3 to 5 start: beside the cover, or on the margin.
    pub fn text_x(&self) -> f32 {
        if self.cover {
            BESIDE_COVER
        } else {
            M
        }
    }

    pub fn strip(&self) -> Vec<(f32, f32, Cell)> {
        if self.cells.is_empty() {
            return vec![];
        }
        strip_layout(&self.cells, self.text_x(), W - M)
    }

    /// What the card says, for a screen reader and the tests.
    /// The name, the outcome and the detail first, then the settings.
    pub fn said(&self) -> String {
        let mut parts = vec![self.name.clone()];
        let out = self.outcome.join(" ");
        if !out.is_empty() {
            parts.push(if self.qualifier.is_empty() { out } else { format!("{out} {}", self.qualifier) });
        }
        if !self.detail.is_empty() {
            parts.push(self.detail.clone());
        }
        // Nothing to say yet: its state, in a word.
        if parts.len() == 1 {
            parts.push(
                match self.shown {
                    Shown::Wait => "waiting",
                    Shown::Busy => "working",
                    _ => "not started",
                }
                .to_owned(),
            );
        }
        for r in &self.rows {
            parts.push(format!("{} {}", r.label, r.value.join(" ")));
        }
        parts.join(", ")
    }
}

/// Engine words that say only that a step hasn't started: before a run
/// the card shows only its name and settings.
fn idle_word(s: &str) -> bool {
    s.is_empty() || s.starts_with("After ") || s.starts_with("Waits for ") || s == "Ready" || s == "Holding" || s == "Holds for you" || s == "Didn't run"
}

/// Works out the card at 100 %.
pub fn build(inp: &Input, measure: Measure) -> Card {
    let v = inp.v;
    let shown = Shown::of(v, inp.retries);
    let mut cut = vec![];
    let text_x = if inp.cover { BESIDE_COVER } else { M };
    let width = W - M - text_x;

    let name = shorten_title(&v.name, W - M - 14.0 - 8.0 - NAME_X, SMALL, 500, measure, &mut cut);

    // Settings: label at 16, value at 96; a single long value may take
    // the second row.
    let value_w = W - M - THIRD;
    let label_w = THIRD - M - 4.0;
    let mut rows = vec![];
    for s in inp.settings.iter().take(2) {
        let lock_by_value = inp.locked && measure(&s.label, SMALL, 400) > label_w - 18.0;
        let room = value_w - if s.yours { 9.0 } else { 0.0 } - if lock_by_value { 14.0 } else { 0.0 };
        let label = shorten_title(&s.label, if inp.locked && !lock_by_value { label_w - 18.0 } else { label_w }, SMALL, 400, measure, &mut cut);
        let value = if measure(&s.value, SMALL, 400) <= room {
            vec![s.value.clone()]
        } else if s.path {
            vec![shorten_path(&s.value, room, SMALL, 400, measure, &mut cut)]
        } else if inp.settings.len() == 1 {
            wrap(&s.value, &[room, room], SMALL, 400, measure, &mut cut)
        } else {
            vec![shorten_title(&s.value, room, SMALL, 400, measure, &mut cut)]
        };
        rows.push(Row { label, value, yours: s.yours, lock_by_value, help: s.help });
    }

    // The outcome and its qualifier; a question asked here is the detail.
    let quiet = idle_word(&v.status) && matches!(v.state, StepState::Idle | StepState::Wait | StepState::Hold);
    let asking = inp.asking || (v.state == StepState::Needs && v.status.ends_with('?') && v.record.started.is_none());
    let (outcome, qualifier, detail) = if asking {
        let d = [v.status.as_str(), v.detail.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        (String::new(), String::new(), d)
    } else if quiet {
        (String::new(), String::new(), String::new())
    } else {
        (v.status.clone(), v.qualifier.clone(), v.detail.clone())
    };
    let qualifier = if qualifier.is_empty() || outcome.is_empty() { String::new() } else { shorten_title(&qualifier, width / 2.0, SMALL, 400, measure, &mut cut) };
    let outcome_lines = if outcome.is_empty() {
        vec![]
    } else {
        let qw = if qualifier.is_empty() { 0.0 } else { measure(&qualifier, SMALL, 400) + 8.0 };
        if measure(&outcome, BIG, 600) + qw <= width {
            vec![outcome.clone()]
        } else {
            wrap(&outcome, &[width, width - qw], BIG, 600, measure, &mut cut)
        }
    };
    let detail = if detail.is_empty() { detail } else { shorten_detail(&detail, width, measure, &mut cut) };
    let working = matches!(shown, Shown::Busy | Shown::Needs | Shown::Wait);

    // Not run yet: a blueprint, only its name and settings. The Drive
    // has something to say before a run (a disc in, or none).
    // Only while it has nothing to say: Not available or Not connected is said.
    let plan = v.record.started.is_none() && v.step != "drive" && !asking && matches!(v.state, StepState::Idle | StepState::Wait) && idle_word(&v.status);
    if plan {
        return Card { shown, name, rows, locked: inp.locked, outcome: vec![], qualifier: String::new(), detail: String::new(), working: false, cells: vec![], cover: false, cut, plan };
    }
    Card { shown, name, rows, locked: inp.locked, outcome: outcome_lines, qualifier, detail, working, cells: cells(v, shown), cover: inp.cover, cut, plan }
}

/// A line as long as fits in `w`, cut with an ellipsis: keeps its start.
fn fit_start(text: &str, w: f32, size: f32, weight: u16, measure: Measure) -> String {
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect();
        if measure(&format!("{}…", s.trim_end()), size, weight) <= w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let s: String = chars[..lo].iter().collect();
    format!("{}…", s.trim_end().trim_end_matches([',', '·', '/', '-']).trim_end())
}

/// A title: whole if it fits, else its start and an ellipsis.
pub fn shorten_title(text: &str, w: f32, size: f32, weight: u16, measure: Measure, cut: &mut Vec<String>) -> String {
    if measure(text, size, weight) <= w {
        return text.to_owned();
    }
    cut.push(text.to_owned());
    fit_start(text, w, size, weight, measure)
}

/// A path: both ends kept, the middle folders given way (*/mnt/nas/…/FLAC*).
pub fn shorten_path(text: &str, w: f32, size: f32, weight: u16, measure: Measure, cut: &mut Vec<String>) -> String {
    if measure(text, size, weight) <= w {
        return text.to_owned();
    }
    cut.push(text.to_owned());
    let parts: Vec<&str> = text.split('/').collect();
    if parts.len() >= 3 {
        let last = parts[parts.len() - 1];
        for k in (1..parts.len() - 1).rev() {
            let s = format!("{}/…/{last}", parts[..k].join("/"));
            if measure(&s, size, weight) <= w {
                return s;
            }
        }
    }
    fit_start(text, w, size, weight, measure)
}

/// The detail line: a time left or a share at its end keeps its place
/// (*Blade Runner · 12 min left*), a path keeps both ends, anything else
/// keeps its start.
fn shorten_detail(text: &str, w: f32, measure: Measure, cut: &mut Vec<String>) -> String {
    if measure(text, SMALL, 400) <= w {
        return text.to_owned();
    }
    if let Some((head, tail)) = text.rsplit_once(" · ").filter(|(_, t)| t.ends_with(" left") || t.ends_with('%') || t.ends_with('×')) {
        let tail = format!(" · {tail}");
        let room = w - measure(&tail, SMALL, 400);
        if room > 40.0 {
            cut.push(text.to_owned());
            return format!("{}{tail}", fit_start(head, room, SMALL, 400, measure));
        }
    }
    if (text.starts_with('/') || text.starts_with("~/")) && !text.contains(' ') {
        return shorten_path(text, w, SMALL, 400, measure, cut);
    }
    // Parts joined by " · " (*SFTRI 577 · Sympathy for the Record
    // Industry*): the last ones whole, the first left out, before cutting
    // into a word. The hover has it all.
    let parts: Vec<&str> = text.split(" · ").collect();
    for k in 1..parts.len() {
        let rest = parts[k..].join(" · ");
        if measure(&rest, SMALL, 400) <= w {
            cut.push(text.to_owned());
            return rest;
        }
    }
    shorten_title(text, w, SMALL, 400, measure, cut)
}

/// Words wrapped into as many lines as `widths` gives, never breaking a
/// word; what doesn't fit in the last line keeps its start.
fn wrap(text: &str, widths: &[f32], size: f32, weight: u16, measure: Measure, cut: &mut Vec<String>) -> Vec<String> {
    let words: Vec<&str> = text.split(' ').collect();
    let mut lines: Vec<String> = vec![];
    let mut k = 0;
    for (row, &w) in widths.iter().enumerate() {
        if k >= words.len() {
            break;
        }
        if row + 1 == widths.len() {
            let rest = words[k..].join(" ");
            let line = if measure(&rest, size, weight) <= w {
                rest
            } else {
                cut.push(text.to_owned());
                fit_start(&rest, w, size, weight, measure)
            };
            lines.push(line);
            break;
        }
        let mut line = words[k].to_owned();
        k += 1;
        while k < words.len() {
            let next = format!("{line} {}", words[k]);
            if measure(&next, size, weight) > w {
                break;
            }
            line = next;
            k += 1;
        }
        if measure(&line, size, weight) > w {
            cut.push(text.to_owned());
            line = fit_start(&line, w, size, weight, measure);
        }
        lines.push(line);
    }
    lines
}

/// Measures with egui's fonts, at 100 %.
pub fn measurer(painter: &egui::Painter) -> impl Fn(&str, f32, u16) -> f32 + '_ {
    move |t: &str, size: f32, weight: u16| painter.layout_no_wrap(t.to_owned(), font(size, weight), Color32::WHITE).size().x
}

impl Card {
    /// The help of the setting whose label is at `at`, in card units from
    /// its top left.
    pub fn help_at(&self, at: egui::Vec2) -> Option<&'static str> {
        let k = SETTINGS.iter().position(|b| at.y >= b.0 && at.y < b.1)?;
        if at.x < M || at.x >= THIRD {
            return None;
        }
        self.rows.get(k)?.help
    }
}

/// A cover to show in the lower-left cell.
pub enum Picture {
    None,
    /// Still loading, or unreadable: the cell, empty.
    Blank,
    Ready(egui::TextureId, [usize; 2]),
}

/// Draws the card into `r` (on screen, `z` its zoom): the slots only; the
/// tile's ground, outline and focus are the canvas's.
#[allow(clippy::too_many_arguments)]
pub fn paint(painter: &egui::Painter, r: Rect, z: f32, c: &Card, v: &StepView, p: &Pal, t: f64, pop: f32, picture: &Picture) {
    let at = |x: f32, y: f32| r.min + vec2(x, y) * z;
    // A line centred on a band.
    let text = |s: &str, x: f32, band: (f32, f32), size: f32, weight: u16, col: Color32| {
        let g = painter.layout_no_wrap(s.to_owned(), font(size * z, weight), col);
        let cy = (band.0 + band.1) / 2.0 * z;
        painter.galley(pos2(r.left() + x * z, r.top() + cy - g.size().y / 2.0), g, col);
    };
    // 1. The kind's icon (a drive that isn't there struck through; a drive
    // reading, its disc turning), the name, the glyph.
    let off = v.step == "drive" && v.status == "Not connected";
    let icon_at = at(M, NAME.0 + 1.0);
    if v.step == "drive" && c.shown == Shown::Busy {
        painter.extend(turning_disc(icon_at, 14.0 * z, p.c_read, t));
    } else {
        let (icon, col) = if off { ("drive-off", p.subtle) } else { (v.step.as_str(), p.kind(&v.step)) };
        painter.extend(kind_icon(icon, icon_at, 14.0 * z, col));
    }
    text(&c.name, NAME_X, NAME, SMALL, 500, p.muted);
    let g = at(GLYPH.x, GLYPH.y);
    if c.shown == Shown::Skipped {
        painter.extend(skipped_glyph(p, g, z * 0.875 * pop));
    } else {
        painter.extend(state_icon(p, c.shown.glyph_state(), g, z * 0.875 * pop, t));
    }

    // 2. Settings: label (tertiary), a lock at the end of its column while
    // ripping, the value (ink) at the first third line, a dot if yours.
    let mut y = 0;
    for row in &c.rows {
        let band = SETTINGS[y.min(1)];
        text(&row.label, M, band, SMALL, 400, p.subtle);
        let mut vx = THIRD;
        if c.locked && row.lock_by_value {
            painter.extend(kind_icon("libcrypt", at(THIRD, (band.0 + band.1) / 2.0 - 5.0), 10.0 * z, p.subtle));
            vx += 14.0;
        } else if c.locked {
            painter.extend(kind_icon("libcrypt", at(THIRD - 8.0 - 10.0, (band.0 + band.1) / 2.0 - 5.0), 10.0 * z, p.subtle));
        }
        if row.yours {
            painter.circle_filled(at(vx + 2.5, (band.0 + band.1) / 2.0), 2.5 * z, p.action);
            vx += 9.0;
        }
        for line in &row.value {
            if y > 1 {
                break;
            }
            text(line, vx, SETTINGS[y], SMALL, 400, p.ink);
            y += 1;
        }
    }

    // The cover, in its cell.
    let x = c.text_x();
    if c.cover {
        let cell = Rect::from_min_max(at(COVER.min.x, COVER.min.y), at(COVER.max.x, COVER.max.y));
        let round = CornerRadius::same((4.0 * z).round().clamp(0.0, 255.0) as u8);
        match picture {
            Picture::Ready(tex, [w, h]) => {
                // Filled, centred: a square cover fills it.
                let (w, h) = (*w as f32, *h as f32);
                let s = (cell.width() / w).max(cell.height() / h);
                let (uw, uh) = (cell.width() / (w * s), cell.height() / (h * s));
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(uw, uh));
                let mut mesh = egui::Mesh::with_texture(*tex);
                mesh.add_rect_with_uv(cell, uv, Color32::WHITE);
                painter.add(Shape::mesh(mesh));
                painter.rect_stroke(cell, round, Stroke::new(z, p.line), StrokeKind::Inside);
            }
            _ => {
                painter.rect_filled(cell, round, p.surface);
                painter.rect_stroke(cell, round, Stroke::new(z, p.line), StrokeKind::Inside);
            }
        }
    }

    // 3. The outcome, its last line on the lower third line, and its
    // qualifier on that line's baseline.
    let n = c.outcome.len();
    for (k, line) in c.outcome.iter().enumerate() {
        let bottom = OUT_BOTTOM - (n - 1 - k) as f32 * OUT_LINE;
        let g = painter.layout_no_wrap(line.clone(), font(BIG * z, 600), p.ink);
        let top = r.top() + (bottom - OUT_LINE / 2.0) * z - g.size().y / 2.0;
        let gw = g.size().x;
        let base = top + baseline(&g);
        painter.galley(pos2(r.left() + x * z, top), g, p.ink);
        if k + 1 == n && !c.qualifier.is_empty() {
            let q = painter.layout_no_wrap(c.qualifier.clone(), font(SMALL * z, 400), p.subtle);
            let qtop = base - baseline(&q);
            painter.galley(pos2(r.left() + x * z + gw + 8.0 * z, qtop), q, p.subtle);
        }
    }

    // 4. The detail: what it's on (ink) or the one fact (secondary).
    if !c.detail.is_empty() {
        text(&c.detail, x, DETAIL, SMALL, 400, if c.working { p.ink } else { p.muted });
    }

    // 5. The item strip.
    let rad = CornerRadius::same((2.0 * z).round().clamp(0.0, 255.0) as u8);
    for (x0, x1, cell) in c.strip() {
        let cr = Rect::from_min_max(at(x0, STRIP.0), at(x1, STRIP.1));
        match cell {
            Cell::Out => {
                painter.rect_stroke(cr, rad, Stroke::new(z.max(0.5), p.faint), StrokeKind::Inside);
            }
            _ => {
                painter.rect_filled(cr, rad, cell_colour(cell, p));
            }
        }
    }
}

pub fn cell_colour(c: Cell, p: &Pal) -> Color32 {
    match c {
        Cell::Wait => p.line,
        Cell::Busy => p.busy,
        Cell::Done => p.subtle,
        Cell::Verified => p.ok,
        Cell::Needs => p.warn,
        Cell::Bad => p.bad,
        Cell::Out => Color32::TRANSPARENT,
    }
}

/// The baseline of a galley's first line, from its top.
fn baseline(g: &egui::Galley) -> f32 {
    g.rows.first().map_or(g.size().y * 0.8, |r| r.pos.y + r.row.glyphs.first().map_or(r.row.size.y * 0.8, |gl| gl.pos.y))
}

/// The disc, turning while the drive reads (still when motion is off): a
/// disc with its hub and a bright sector going round.
pub fn turning_disc(at: Pos2, size: f32, col: Color32, t: f64) -> Vec<Shape> {
    let k = size / 24.0;
    let c = at + vec2(12.0, 12.0) * k;
    let st = Stroke::new(1.8 * k, col);
    let a0 = if crate::motion::on() { (t * std::f64::consts::TAU / 1.6) as f32 } else { 0.0 };
    let arc: Vec<Pos2> = (0..=8).map(|i| c + egui::Vec2::angled(a0 + i as f32 / 8.0 * 1.4) * 5.5 * k).collect();
    vec![Shape::circle_stroke(c, 9.0 * k, st), Shape::circle_stroke(c, 2.0 * k, st), Shape::line(arc, st)]
}

/// Skipped: a grey disc with a cross.
fn skipped_glyph(p: &Pal, c: Pos2, z: f32) -> Vec<Shape> {
    let s = Stroke::new(1.8 * z, Color32::WHITE);
    let pt = |x: f32, y: f32| c + vec2((x - 8.0) * z, (y - 8.0) * z);
    vec![Shape::circle_filled(c, 7.0 * z, p.faint), Shape::line_segment([pt(5.5, 5.5), pt(10.5, 10.5)], s), Shape::line_segment([pt(10.5, 5.5), pt(5.5, 10.5)], s)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_strip_merges_cells_into_ones_at_least_three_wide() {
        let cells = vec![Cell::Done; 99];
        let s = strip_layout(&cells, 16.0, 272.0);
        assert!(s.len() < 99 && s.len() > 40, "{}", s.len());
        assert!(s.iter().all(|(a, b, _)| b - a >= MIN_CELL - 0.01));
        // The most urgent member wins.
        let mut cells = vec![Cell::Done; 99];
        cells[50] = Cell::Needs;
        assert!(strip_layout(&cells, 16.0, 272.0).iter().any(|c| c.2 == Cell::Needs));
        // 17 fit with 2 between.
        let s = strip_layout(&[Cell::Wait; 17], 16.0, 272.0);
        assert_eq!(s.len(), 17);
        assert!((s[1].0 - s[0].1 - 2.0).abs() < 0.01);
        assert!((s[16].1 - 272.0).abs() < 0.01);
    }

    #[test]
    fn paths_keep_both_ends_and_titles_their_start() {
        // A fake font: every character 7 wide.
        let m = |t: &str, _: f32, _: u16| t.chars().count() as f32 * 7.0;
        let mut cut = vec![];
        assert_eq!(shorten_path("/mnt/nas/media/music/lossless/FLAC", 15.0 * 7.0, 13.0, 400, &m, &mut cut), "/mnt/nas/…/FLAC");
        assert_eq!(shorten_title("The Rise and Fall of Ziggy", 10.0 * 7.0, 13.0, 400, &m, &mut cut), "The Rise…");
        assert_eq!(cut.len(), 2);
        assert_eq!(shorten_path("~/Music/FLAC", 200.0, 13.0, 400, &m, &mut cut), "~/Music/FLAC");
        assert_eq!(cut.len(), 2, "what fits isn't cut");
    }
}
