# Quorum: interaction and feel

The window draws well, but it doesn't answer you. Things fade at most, nothing moves, nothing is pressed or focused, and a running pipeline looks like a resting one.

## Diagnosis

1. **No motion system.** `animate_` appears nowhere in `src`.
   - The only time-driven things are the spinner (`theme.rs:172-180`) and egui's default `Area` fade, which is 200 ms with `quadratic_out` and covers the drawer and the popover.
   - Nothing reads reduced motion, though the mockup honours it (`mockups.html:130`).
2. **No press state.** `Btn`, tiles, rows, tabs and segments paint hover only (`widgets.rs:89-108`). Only the window buttons show a press (`chrome.rs:91`).
3. **No focus ring.** egui already moves focus between tiles with Tab and the arrow keys (`memory/mod.rs:590`), but nothing paints `has_focus()`, so that focus can't be seen.
4. **The drawer and popover fade in but don't travel.** They appear at their final rect (`tray.rs:55-66`, `popover.rs:119`). When a question opens the drawer by itself (`app.rs:359-363`), nothing ties it to the tile that asked.
5. **The camera teleports.** ±, 100%, Fit (`app.rs:990-1001`) and `reveal` (`canvas.rs:41-49`) all snap.
6. **Running looks like resting.**
   - Wires are always `p.faint` (`canvas.rs:132`).
   - Progress jumps from snapshot to snapshot, and the bar is square on a rounded tile (`canvas.rs:326-329`; see the `running` shot).
7. **Changes are silent.** Done, Bad and Needs only swap colour. A question that arrives while the window is in the background gives no urgency hint.
8. **Smaller frictions.**
   - The popover can light two rows at once (`popover.rs:183-187`).
   - Dragging shows no grabbing cursor (`canvas.rs:82`).
   - A loading cover and a failed one look the same (`tray.rs:2280-2286`).
9. **Repaints ignore visibility.** Anything Busy repaints the whole UI every 33 ms (`app.rs:450`). A drive in use does the same (`tray.rs:2234`). Both continue while the window is unfocused, for hours of encoding.

## Ground rules

- **One helper.** `tween(ctx, id, on: bool, secs, ease) -> f32` wraps `animate_bool_with_time_and_easing`.
  - It returns the target at once unless `app.motion`, which is `!reduced && self.clock.is_none()`. That keeps screenshots deterministic.
  - egui returns the target the first time it sees an id. So call it **every frame**, with the condition (for example `tray.is_some()`), not only once something has appeared.
- **Reduced motion.**
  - **GNOME:** read `org.gnome.desktop.interface enable-animations` via the Settings portal (`ReadOne`, then watch `SettingChanged`). This works in the flatpak.
  - **macOS:** `accessibilityDisplayShouldReduceMotion`.
  - **Windows:** `SPI_GETCLIENTAREAANIMATION`.
  - **When reduced:**
    - Set `style.animation_time = 0`, which also kills egui's fades.
    - Nothing slides, scales, shakes or marches, and colour still changes.
    - The spinner becomes a still ¾ arc.
- **Idle is idle.** Each cue records when it ends. Repaint every 16 ms only while a cue is running. The only loops are the spinner and the wire march, and both run only while Busy.
- **Cues play once,** because a question or hold can wait for hours.
- **Cues are keyed by (recipe, run id, tile).** The first frame after switching recipe or run records states without playing them.

## Proposals

**P1. Press, hover fade, focus ring.** High impact, M effort, behaviour.
- **Where:** `Btn::show`, `segmented`, `tabs`, tiles (`canvas.rs:183`), recipe rows (`app.rs:545`) and popover rows.
- **Hover:** the fill fades in over 90 ms with `cubic_out`.
- **Press:**
  - The fill becomes `mix(ink, 0.10, base)`, or `mix(ink, 0.22, action)` for Primary.
  - The label drops 1 px.
  - A tile insets by 1 × z.
- **Focus ring:**
  - 2 px in `p.action`, `StrokeKind::Outside`, offset 2 px, with the widget's radius plus 2.
  - It shows only while `app.kb_focus` is set: Tab or an arrow key sets it, and a pointer press clears it.
  - A tile that gains focus calls `reveal`.
- **Esc:** `app.rs:372` treats any focus as typing, which would make the first Esc on a focused tile do nothing. Replace that with `app.text_focus`, set by the text fields themselves.

**P2. The drawer slides up.** High, S, taste.
- **Where:** `tray.rs:55-66`.
- **Motion:**
  - Paint at `rect.translate(0, (1-k)·h)`, where `k = tween(.., tray.is_some(), 0.18, cubic_out)`.
  - Scale the shadow alpha by `k`.
  - Keep egui's fade.
  - The height never tweens, so `request_discard` (`app.rs:966`) stays quiet.
- **No slide** when you switch tiles with the drawer open, or when it closes.
- **When a question opens it:** the asking tile's outline grows from 0 to 2 px over the same 180 ms.

**P3. The camera glides.** High, M, taste.
- **Where:** add `glide: Option<(usize, View, View, t0)>` to `App`, stepped in `stage()`.
- **Timing:**
  - Fit and 100%: 240 ms, `cubic_in_out`.
  - ±: 140 ms, `cubic_out`.
  - `reveal`: 180 ms, alongside the drawer.
- **Path:** zoom moves geometrically (`z0·(z1/z0)^k`) and pan linearly.
- **Interrupting:** a drag or scroll cancels the glide.
- **The popover:** it closes on any view change (`popover.rs:59`), so never start a glide while it's open.

**P4. Touchpad pans, wheel zooms.** High, S, behaviour.
- **Why:** heuristic 4.3 kept "every scroll zooms" on purpose. On Linux egui-winit reports the wheel source as `Unknown`, so branch on the unit instead.
- **Where:** `canvas.rs:86-93`. Read `Event::MouseWheel { unit, .. }`.
  - `MouseWheelUnit::Point` (touchpad pixel deltas): pan, both axes.
  - `Line` or `Page`: zoom, as now.
  - Pinch and Ctrl+scroll: zoom.
- **Cursor:** `Grabbing` while dragging.
- **Test first:** check a hi-res wheel on GNOME before shipping.

**P5. Tiles acknowledge a change, once.** High, M, taste.
- **Done:**
  - The icon scales from 0.6 to 1.0 over 240 ms with `back_out`.
  - The tint goes from `mix(ok, 0.18)` back to its resting 0.07 over 600 ms with `cubic_out`.
- **Bad:** the same in `p.bad`, without the overshoot.
- **Needs:** a single 2 px `p.warn` ring grows 0→8 × z while fading out over 700 ms.
- **Sidebar:** a row that isn't selected gets the 600 ms wash when it turns amber or green.

**P6. Progress glides.** Medium, S, taste.
- **Where:** `canvas.rs:326-329`, and the bars on the drawer's items.
- **Motion:** move to each new value over 300 ms with `cubic_out`. Snap if the value drops by more than 1%, because that means a new item.
- **Shape:** round the bar's bottom corners to the tile's.

**P7. Wires carry the flow.** High, M, taste.
- **Into a Busy tile:** `p.busy` at 70%, with 6/6 × z dashes marching toward the tile at 24 × z px/s.
- **Between two Done tiles:** `mix(ok, 0.35, faint)`.
- **Reduced motion:** colour only.
- **Cost:** no new wakeups.
- **Needs:** step ids added to `Wire` (`crates/layout/src/lib.rs:90`).

**P8. Repaint budget.** Medium, S, behaviour.
- **Where:** `app.rs:450` and `tray.rs:2234`.
- **Rates:**
  - Focused: every 33 ms.
  - Unfocused: every 250 ms.
  - Minimised or reduced motion: no timer. The snapshot watcher (`model.rs:524`) still wakes the window when a run changes.
- **Rely on unfocused:** Wayland may never report minimised, so the unfocused rule does most of the work.

**P9. Ask for attention.** Medium, S, behaviour.
- **When:** a question appears and the window isn't focused.
- **What:** send `RequestUserAttention(Informational)` once per key from the ground rules (`app.rs:359`). *Tell me* owns notifications.

**P10. The popover: one highlight.** Medium, S.
- **Behaviour:**
  - Hover sets `pop.sel` (`popover.rs:183-188`).
  - Add Home and End.
- **Taste:**
  - The popover slides 4 px from the + side over 120 ms, on top of egui's fade.
  - The + turns 45° into a × while its popover is open, over 160 ms (`canvas.rs:225`).

**P11. Zoom from the keyboard.** Medium, S, behaviour.
- **When:** only while no text field has focus.
- **Keys:** Ctrl+= and Ctrl+- zoom, Ctrl+0 goes to 100%, and Shift+1 fits, all gliding.
- **Tiles:** egui already moves between tiles with the arrows and clicks with Enter or Space, so only the ring (P1) is missing.

**P12. Loading, failure, refusal.** Low, S, taste.
- **Loading cover:** a small spinner.
- **Failed cover:** the disc glyph in `p.faint`.
- **Refusal:** the red sentence (`tray.rs:593`) shakes once, ±4 px decaying over 280 ms, as GNOME does for a wrong password.

## Outside my lane
- The popover hides the tiles to the right of its +.
