# Beautifying pass: what the quorum agreed

Three reviews, read together: `quorum-visual.md` (craft), `quorum-layout.md`
(hierarchy), `quorum-interaction.md` (feel). What two or more of them
point at, or what plainly follows the user's stated preferences, is
**built**. What changes behaviour or is a matter of taste is **asked**.

## Built

**The running thing is the loudest thing.** All three found the canvas
upside down: done tiles shout in green, waiting ones in orange, and the
tile actually working is the quietest.
- Orange means *needs you* and nothing else; waiting (*Waits for
  choose*, *Drive busy*) is grey. (visual 1, layout 1)
- Done tiles keep a light tint and the tick, but their status drops to
  ink at a normal weight; busy, needs-you and failed tiles get a 2 px
  outline in their colour. (layout 1, visual)
- Wires into a busy tile turn the busy colour, with dashes moving toward
  it; without motion, just the colour. (interaction 2)

**One of everything.**
- One state pill, 20 px, everywhere (header and drawer). (visual 2)
- One chevron for dropdowns, menus and disclosures, drawn like the
  app's icons, replacing egui's solid ▼, ▾ and triangle. (visual 4)
- One drawer anatomy: settings column on the left, the work on the
  right, height from its content up to the user's chosen height. (layout 4)

**Craft.**
- Focused fields: a 1.5 px border in the action colour; selected text
  tinted with it. (visual 3)
- Small text meets 4.5:1: a new token for small muted text. (visual 5)
- Dark mode gets step colours that read on dark (Choose's icon is
  2.3:1 today). (visual 5)
- *default* is shown only where a value isn't the default; the line
  stays for *yours*, *AccurateRip, 528 agree*, *app-wide*. (layout 5;
  also the heuristic review's open question, settled the quieter way)

**Feel.**
- Every clickable thing has hover, press and a visible keyboard focus
  ring; Tab and the arrows already move between tiles, now you can see
  where. (interaction 1)
- One-shot cues: a tile that finishes pops its tick and fades its tint
  in; a question rings once. Never looping, never replayed when you
  switch recipes. (interaction 3)
- The drawer slides up (180 ms); Fit, ± and centring on a tile glide.
  (interaction 4)
- The opened tile is centred above the drawer with its neighbours in
  view. (layout 3)
- Reduced motion (GNOME's *Reduce animations*) turns all of this into
  instant changes. Idle stays idle: repaint only while something moves,
  and slowly when the window isn't focused.

## Asked

1. **A view that follows the rip.** Open fitted to the width and pan
   right as the work moves (today the encoders and Library are off
   screen at 1280 × 720). Changes the canvas from staying where you put
   it to following along.
2. **Touchpad two-finger swipes pan; the wheel zooms.** You asked for
   scroll to zoom; a touchpad's swipe is also a scroll. Linux can't
   always tell them apart.
3. **A primary button colour of its own**, separate from the blue that
   means *running*.
4. **Zoom controls in the header** instead of floating on the canvas.
