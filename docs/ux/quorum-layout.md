# Quorum: layout and visual hierarchy

Lens: where the eye goes, how the window divides, tile and drawer anatomy. Shots are in `app/shots/`. All measurements are at 100% zoom, in logical px.

## Diagnosis

1. **Done tiles are louder than the running one.** Mid-rip, every finished tile has a green tint and a 15/700 green status ("17 tracks", "2 releases", "Found"). The only white tile is the busy one, so the step you should be watching reads as the *least* finished and the least emphasised (`running-light`, `encoder-light`). The cause is `tile_fill` and the status sizes in `canvas.rs:259-317`.
2. **The default view shows the wrong half of the recipe.** It opens at zoom 1.0, pinned top-left (`app.rs:174`, `canvas::MARGIN`). At 1280×720 the canvas stops at Rip, so the encoders, Hold and Library (the outcome) are off-screen. Meanwhile the bottom 45% of the canvas is empty dots (`running`, `chrome`, `ask`). At 1920 it still clips Library (`scale-step`).
3. **Opening a drawer drops the context around the tile.** `View::reveal` (`canvas.rs:41-50`) pans only as far as needed, with a 24 px margin, so the open tile ends up against an edge and its neighbours are cut. Examples: `drawer-rip` (FLAC's name is cut off), `hold-dark` (the tile after Hold shows as a lone "W"), `drawer-encoder`.
4. **The zoom pill floats over content.** It is pinned 12 px above the drawer (`app.rs:946-947`), so it sits on the CD-Text tile (`setup-light`, `drawer-library-held`) and across the loop wire (`ask`, `hold`).
5. **The drawer is always half the window, whatever it holds.** `tray.rs:198` clears `drawer_short` whenever there's a body, so the drawer always fills `max` (50%). In `inside` and `drawer-rip` it holds one item row and about 200 px of nothing, while it hides the tiles above it.
6. **The drawer has three different skeletons.**
   - Items: a 248 px settings column beside a list (`drawer-rip`, `drawer-encoder`).
   - No items: a grid 3–5 columns wide (`drawer-mp3-vbr`, `scale-step`).
   - Choose: its own layout (`ask`).

   Settings jump from left-stacked to spread across the drawer as you click along the strip (`tray.rs:166-176`).
7. **The settings column is mostly provenance.** Every field stacks label, value and then "default" (`tray.rs:676-686`). `drawer-mp3-vbr` has six "default"s. A locked field is three lines of plain text about 50 px tall that looks like a broken form (`drawer-rip`: "AccurateRip / On / default").
8. **The idle tile's big slot is wasted.** It says "After Audio CD" or "After all 2" in the status position (`loop`, `scale-step`), which repeats what the wires already show. Line 2 is often a generic kind word ("lookup", "table of contents", "which release") rather than configuration.
9. **The status is said four times; the header pill carries the least.** It says "Running" (`running`) where the mockup said "about 4 min left". Its case drifts too: "holds for you" against "Holds for you" (`drawer-library-held`, `hold`).
10. **Sidebar rhythm.** The drive repeats on every row (four times in `running`). An earlier disc has its own grammar, cut to "The White St…" (`next-disc`). "Dark" is orphaned at the bottom left.

Glaring, outside my lane: after Stop, Cover art and Rip show a green "Waits for choose" with check marks (`chrome-light`).

## Proposals

### P1. Make the busy, needed and failed tiles the loudest
- **Done tiles:**
  - Fill: `raised` (drop the `mix(ok,0.07)` in `tile_fill`, `canvas.rs:259`).
  - Status: 14/600 in `ink`.
  - The green check stays as the done signal.
- **Busy, Needs and Bad:** keep 15/700 in their colour, and add a 2 px state-coloured outline.
- **Needs:** keeps its `warn_soft` fill.

Impact high, effort S. **Taste:** it departs from the approved mockup's green tint.

### P2. A frame that follows the rip
- **On selecting a recipe, or starting a run,** zoom to fit the width of the layout:
  - zoom = clamp((canvas_w − 64) / lay.width, 0.75, 1.0);
  - centre it vertically in the *top 55%* of the canvas, the band the drawer never covers.
- **While running,** if the frontier (the right-most Busy or Needs tile) leaves that band, pan so it sits 2/3 of the way across. Ease the pan over 200 ms. Stop following after any manual pan until the next disc.
- **Where:** the initial `View` (`app.rs:174/922`); a new `View::frame` beside `fit` (`canvas.rs:52`).

Impact high, effort M. **Behaviour:** the user decides whether the canvas may move by itself.

### P3. Centre the opened tile, not just reveal it
- Replace `reveal` (called from `app.rs:907-913`) for drawer opens. It should centre the tile horizontally and centre it vertically in `canvas_h − drawer_h`.
- If the tile is in a strip, centre the whole strip, so the neighbours stay whole.
- Keep a minimum margin of 48 px.

Impact high, effort S. **Taste.**

### P4. Move the zoom controls into the header
- Put "− 100% + ⤢" as 28 px ghost buttons in the header's right cluster, 12 px left of Edit, behind a 1 px × 20 px `line` divider.
- Delete the floating Area (`app.rs:946-947, 976-1003`). The canvas then has nothing floating over it.

Impact medium, effort S. **Taste.**

### P5. Size the drawer to its content
- Height = clamp(natural, 220, 55% of canvas). The tray already measures `drawer_natural`; stop clearing `drawer_short` at `tray.rs:198`.
- The list scrolls past the cap, as now.
- With P3, a one-row Rip drawer becomes about 220 px and the canvas gains about 110 px.

Impact medium, effort S. **Behaviour (small):** the drawer changes height between steps.

### P6. One drawer skeleton
- **Header, 56 px:** icon and name 17/700, then the pill, then the sentence (13/400 `muted`), then a flexible space, then a ghost secondary action, the primary button, ⤢ and ×.
- **Body:** always two regions.
  - Settings rail: fixed at 280 px, `surface` background, 20 px padding.
  - Main region: the flexible rest.
- **A step with no item list** (MP3, Library at setup) puts its settings *in the main region*, as a grid of 240 px columns with a 24 px gap. The rail then holds just "Locked while ripping" or the step's one-line explanation.
- **Choose** follows the same skeleton: the rail is "Found by…", the main region is the candidates and the tracks.

- **Where:** `tray.rs:166-185`. Replace the three-way `left_w` with a fixed `RAIL = 280.0`.

Impact high, effort M. **Taste.**

### P7. Show provenance only when it says something
- A default value shows no caption. Only "yours, 18 Nov", "AccurateRip, 528 agree" and Reset show, 12 px `faint`, 2 px under the control.
- Field rhythm: label 12.5, 4 px, control, 14 px gap. That saves about 18 px per field.
- **Locked fields** become a two-column definition list in 26 px rows: the label in `muted` on the left, the value in `ink` 13/500 right-aligned at the rail's right edge. Above it goes a 14 px lock icon and "Locked while ripping" as the section head.
- **Where:** `field`, `field_reset` and `provenance` (`tray.rs:676-720`), plus the locked branch in `settings` (`tray.rs:761`).

Impact medium, effort S. **Behaviour (small):** "default" disappears.

### P8. Sidebar
- **Recipe row:** two lines.
  - Name: 14.5/600.
  - Status: 13/400.
  - Height: 52 px, down from 78.
- **Drives:** with more than one drive, group the recipes under drive headings (11/600, uppercase, 0.04 em tracking, `faint`, 16 px above the group). With one drive, there are no headings.
- **Earlier discs:**
  - 28 px rows, the album text starting at x = 45 (aligned with the recipe name).
  - The state as an icon only, in the same x = 207 column as the recipe icons.
- **Theme toggle:** becomes a 28 px icon button (sun or moon) at the bottom left, 12 px inset.
- **Where:** `app.rs:466-615`.

Impact medium, effort M. **Taste.**

### P9. The header pill carries the run, and the tiles carry the steps
- Running: "about 4 min left · 6 of 17".
- Needs you, failed, held: the count and the noun ("1 to retry", "17 held").
- Idle: no pill.
- Pills always use sentence case.

Impact medium, effort S. **Taste.**

### P10. Readable at Fit
Below zoom 0.7, the tiles switch to a glance form (`tile_body`, `canvas.rs:299`):
- line 2 and the detail are dropped;
- the name is drawn at max(13.5z, 10) and the status at max(15z, 11);
- the state icon is kept.

`complex-fit` at 44% currently has unreadable 6 px text.

Impact medium, effort S. **Taste.**

## Tile anatomy (152 × 112, unchanged)

```
┌──────────────────────────────┐  2px state outline if busy/needs/bad
│ ◉ Rip                    ◌   │  y12  icon16 + name 13.5/600 · state icon 16
│ cyanrip, AccurateRip         │  y30  config 12/400 muted — never a kind word;
│                              │       empty if there's no config
│                              │
│ 6 of 17                      │  bottom-46  status 15/700 state colour
│ Cannon · about 2 min left    │  bottom-26  detail 12/400 muted, the tail kept
│▀▀▀▀▀▀▀▀▀▀▀░░░░░░░░░░░░░░░░░░│  bottom 4px progress
└──────────────────────────────┘
done: same anatomy, raised fill, status 14/600 ink, ✓ top right
idle: status slot empty (no "After …"); a hold shows "Holds for you" in warn
```

## Drawer structure

```
┌ grip 40×4 ──────────────────────────────────────────────────────────────┐
│ ↓ Rip  [✕ 1 to retry]  16 verified        Working ▢   [Read again]  ⤢ × │ 56
├───────────────────────┬─────────────────────────────────────────────────┤
│ 🔒 Locked while ripping│ [Needs you 1] Verified 16  All 17   offset +103 │
│ AccurateRip        On │ 9  Broken Bricks   1:51  ✕ Doesn't match  …     │
│ First read       Fast │                                                 │
│                       │                                                 │
│ rail 280, surface     │ main: list, or the settings grid (240 + 24 gap) │
└───────────────────────┴─────────────────────────────────────────────────┘
height = clamp(content, 220, 55% canvas); the list scrolls inside
```
