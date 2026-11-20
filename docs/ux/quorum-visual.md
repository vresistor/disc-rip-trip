# Quorum: visual craft

The palette (`theme.rs:45-58`) and the half-pixel type scale both match `mockups.html` exactly. Each item is marked **drift** (the app fell short of the mockup) or **shared** (the mockup has the flaw too, so changing it is the user's call). Contrast figures are computed WCAG ratios: 4.5 for small text, 3.0 for icons and large bold text.

## Diagnosis

1. **Orange is everywhere, so "needs you" doesn't stand out.** Wait and Hold paint their status in warn at 14/700 with an orange clock (`canvas.rs:313`, `theme.rs:188`). `locked-flac-dark` has eight orange "Waits for choose" tiles. `ask-light` has two beside the one tile that asks; the `running-light` sidebar has three orange "Drive busy" rows. *Shared.*
2. **The header pill is a stretched blob.** `chip()` (`widgets.rs:123`) fills the header row's `set_min_height(34.0)` (`app.rs:698`), so it renders 34 px tall and fully round, against 18 px in the mockup (`running-*`, `ask-light`, `chrome-light` "Stopped", `quit-confirm-light`). It is also a second pill design (12/500, no icon) beside the drawer's `pill()`. *Drift.*
3. **Stock egui shows through.** The ComboBox's solid ▼ (`drawer-mp3-*`, `drawer-per-item-light`, `drawer-folder-template-light`; `tray.rs:961,1171,2495`). The "More ▾" `menu_button` (`drawer-rip-light`, `tray.rs:1376`), which looks like none of the `Btn` looks. The collapsing triangle on "Advanced" (`locked-flac-dark`). *Drift.*
4. **Focus and selection are broken in the visuals.** `selection.stroke = action_ink` (`theme.rs:139`) is egui's focused-TextEdit border: white in light, #0f1520 in dark. The focused Folder field in `setup-light` has no visible edge, and selected text sits on solid #2348b8. *Drift.*
5. **Faint text fails contrast.** `faint` #8c95a1 on surface is 2.9:1, and it sets 12 px drive names, "default" captions, durations and track numbers (`drawer-mp3-light`, `ask-light`). In dark, #707b88 on raised is 3.36. The idle ring `pending` is 1.82 in light and 1.77 in dark, so the rings vanish in `drawer-mp3-dark`. *Shared.*
6. **Dark mode borrows light-mode colours.** `..Pal::light()` (`theme.rs:59`) keeps the light kind colours. The Choose icon (`c_chk` #17784a) sits at 2.31:1 on an Ok tile and is nearly invisible in `running-dark` and `hold-dark`; `c_read` is 2.74. `action == busy` (#7d9cf2): selection ring, progress, Running pill and primary button are one colour. The pastel primary ("Let 17 through" in `hold-dark`, Save in `picker-popover-dark`) reads as disabled. *Shared.*
7. **The type scale is mush.** The code uses 19 size/weight pairs across 8 sizes from 11.5 to 17, in half-pixel steps. Each setting in `drawer-mp3-light` stacks three greys at 12.5, 13.5 and 12. The drawer title is 17 (`tray.rs:586`), the same as the window title, where the mockup has 16. *Partly drift.*
8. **Floating things have no lift.** The zoom control (`app.rs:984`) has no shadow, so tiles slide under it and it reads as part of them: the CD-Text tile in `setup-light`, the loop arrow running through it in `ask-light`. *Drift:* the mockup's z-index plus scrollbar separated them.
9. **Icons look ragged next to the type.** egui polylines have butt caps and sharp joins. At 16 px, 1.8/24 comes out at a 1.2 px stroke, so the tick in Choose, the Rip arrow and the Folder strokes look chipped against the mockup's round caps. Beside them, the solid 14 px state discs outweigh the kind icon and the name. *Drift.*
10. **Zoomed out is unreadable.** At 44% in `complex-fit-light`, text is 6 px and borders are `Stroke::new(z,…)` at 0.44 px (`canvas.rs:193,208`). *Drift.*

Other lanes, glaring: the bare "Dark" word at the sidebar foot; "Waits for you" vs "Holds 17 for you" (`hold-dark`); "default" under every setting.

## Proposals

**P1. Orange means you (High, S, a mild departure).** Wait: status in `muted` at 14/600, with the clock in `muted` (`canvas.rs:313`, `theme.rs:188`). Hold keeps warn. Sidebar "Drive busy" goes to `muted` text and a `muted` clock. Only Needs and Hold stay orange.

**P2. One pill (High, S).** Delete `chip()` from the header (`app.rs:738-763`) and use `pill()` with its state icon. Make `pill()` exactly 20 px tall, padding 8 left and 10 right, radius 10, at 12.5/600. Keep `chip()` for provenance tags only, and make it allocate its exact size like `pill()` does so it can't stretch.

**P3. Focus and selection (High, S).** `theme.rs:138-139`: `selection.bg_fill = action @ 22% alpha` (light: rgba(35,72,184,56)); `selection.stroke = Stroke::new(1.5, action)`. Text fields at rest get a 1 px `line` border and `surface` fill.

**P4. Replace the stock widgets (High, M).** Add a `"chevron"` glyph: `(6,9)→(12,15)→(18,9)` at 1.8. Then:
- ComboBox: `.icon(|ui, r, v, open| …)` paints the chevron 12 px in `muted`, turned 180° when open.
- "More ▾": `Btn::new("chevron","More").look(Quiet)`, icon after the label.
- "Advanced": the same chevron, turned −90° when closed.

**P5. Text-contrast tokens (High, S, taste).** `faint` stays as it is for wires and hover strokes (`canvas.rs:132`, `theme.rs:148`), which aren't text. Add `subtle` for 12–12.5 px text: light #6b7480 (4.53 on surface), dark #8693a0 (4.62 on raised). Move all 29 `p.faint` text uses in `tray.rs` to it. Darken `muted` to keep three distinct steps: light #4c5563 (7.2), dark #b3bcc7 (7.5). `pending`: light #8a939f (3.11), dark #707b88 (3.36).

**P6. A dark palette of its own (High, M, taste).** `theme.rs:52-60`:

| Token | Dark now | Proposed | On raised |
|---|---|---|---|
| c_src | #6b7a8f | #8e9cb0 | 5.2 |
| c_read | #3b6fe0 | #6f95f0 | 5.0 |
| c_look | #8a5cd0 | #a983e6 | 4.9 |
| c_make | #c27a1a | #dd9a45 | 6.1 |
| c_put | #2f8a63 | #4fb385 | 5.6 |
| c_chk | #17784a | #5cc48e | 6.7 |
| action (fill) | #7d9cf2 + #0f1520 ink | #3d63d6 + #ffffff ink (5.3) | — |
| action_text (new) | — | #9db4f7, for links, Reset and the tile selection ring | 7.1 |

`busy` stays #7d9cf2, so in dark running and selected are different colours.

**P7. Six text roles (Medium, M, taste).** Add consts in `theme.rs` and map every `font()` call to one of them (table below). Titles get `extra_letter_spacing −0.2`; Micro gets +0.3 (both are `TextFormat` fields in egui 0.37). The drawer title drops to 16 (`tray.rs:586`), as in the mockup.

**P8. Elevation scale (Medium, S).**
- e1, the zoom control: `Shadow{offset:[0,2], blur:8, color: black α18 light / α64 dark}`.
- e2, the popover: unchanged (`popover.rs:128`).
- e3, the drawer: unchanged (`tray.rs:123`).

**P9. Icon finish (Medium, S).** In `glyph()`'s `l`/`tick` closures (`theme.rs:231,236`), add `circle_filled(vertex, w/2)` at every vertex to give round caps and joins. Use 2.0/24 when `size ≤ 16` (1.33 px). Shrink the state disc from r 7 to r 6.5 (`theme.rs:167`).

**P10. Legible when small (Medium, M).** Floor tile borders and notches at `max(z, 1.0)` and wires at `max(2z, 1.25)`. Below z 0.6, draw only icons and the name at `max(13.5z, 10)` px (`canvas.rs:300-325`).

**P11. Mono where columns matter (Low, S, taste).** Bundle JetBrains Mono 400 as `Monospace` (`theme.rs:118`, now aliased to Schibsted). Use it at 12.5 for durations (`tray.rs:1469,2431`), the offset, and the template field and its token chips (`tray.rs:1190,1215`). The mockup set durations in mono.

**P12. Segmented control (Low, S, taste).** Radius 7 → 6, to match `Btn` (`widgets.rs:202,220`). In dark, the chosen cell's fill changes from `ink`, a near-white slab in `drawer-mp3-dark`, to `mix(ink, .14, raised)` with ink text and a 1 px `faint` edge.

## Tokens: now and proposed

**Text colours (light / dark)**

| Token | Now | Proposed |
|---|---|---|
| ink | #18202b / #e6eaef | unchanged |
| muted | #5b6573 / #a2acb8 | #4c5563 / #b3bcc7 |
| subtle (new, small text) | (faint) #8c95a1 / #707b88 | #6b7480 / #8693a0 |
| faint (lines only) | #8c95a1 / #707b88 | unchanged |
| pending | #b9c1cb / #46505c | #8a939f / #707b88 |

**Type**

| Role | Now (in use) | Proposed | Used for |
|---|---|---|---|
| Micro | 11.5/500, 11.5/600 | 11.5/600, +0.3 | group labels ("Look up", "Tags and art") |
| Small | 12/400–500, 12.5/400–600 | 12.5/400 (500 for marks) | from and detail lines, captions, pills |
| Body | 13/400–500, 13.5/400–500, 14/400–500 | 13.5/400; buttons 13.5/500 | rows, values, buttons |
| Strong | 13.5/600, 14/600, 14.5/600 | 14/600 | tile and sidebar names, row titles that need attention |
| Status | 14/600–700, 15/700 | 15/700 (Idle and Wait 14/600) | tile status |
| Title | 15/700, 17/700 | 17/700, −0.2 (drawer 16/700) | window, drawer, "Recipes" (15 → 14/600 Strong) |

**Spacing**

| Use | Now | Proposed |
|---|---|---|
| Base steps | 2, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18 | 4, 8, 12, 16, 24 (and 2 for hairline nudges) |
| Pill padding | 7 + 9, chip 8 × 1 | 8 + 10, height 20 |
| Radii | 4, 6, 7, 8, 10, 14, 99 | 6 (controls), 8 (cards, zoom), 10 (tiles, popover, window), 14 (drawer), full (pills) |
