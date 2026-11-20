# Quorum: where a step's settings are edited (space and form)

Lens: space and form. The question is what stays visible, how far the eye and the pointer travel, how the form copes with 0 to 6 settings, and how it looks. Mockup: [config-quorum-space.html](config-quorum-space.html), at 1280 × 720 and 100 %, on the CD recipe. Each option is shown with FLAC, Rip and Library selected, and with Rip mid-rip.

## What's wrong with the drawer for settings

- **Two jobs on one surface.** Setting a step up and watching it run are separate tasks, done at different times. The rail puts them side by side: a 192 px column of settings beside the run (`record-files`). Before a run, the right-hand side is empty. Mid-run, the rail is locked and dead (`drawer-rip`).
- **It's far from what it edits.** The value sits on the card at the top of the canvas, and its control sits at the bottom left of the window. For FLAC, *8* on the card is about 500 px from the slider (`setting-reset`). Library's folder is about 530 px away.
- **It hides the recipe to show very little.** Half the canvas (1048 × 335) goes on one segmented control for Rip. Above it the canvas holds one strip, so the branch below the card is cut off (`drawer-folder-template`).
- **Three skeletons.** A rail beside a list, columns across the width (`drawer-mp3`), and Choose's own layout. `quorum-layout.md` item 6 flagged this.

## Measured at 1280 × 720 (canvas 1048 × 670)

| | Canvas left showing | Cards across | Pointer, value on card → its control | Card and neighbours |
|---|---|---|---|---|
| Today: drawer with a rail | 1048 × 335 | 3.6, one strip high | 370–530 px, diagonal | card whole; the strip below is cut |
| 1. Card expands in place: its face becomes the form (mock C) | 1048 × 670 | 3.6 | 0–20 px | card changes size; the strip below moves |
| 2. Inspector on the right (mock B) | 696 × 670 | 2.4 | 350–790 px, sideways | all whole, but fewer of them |
| 3. Popover from the card | 1048 × 670 | 3.6, one or two covered | 100–250 px | covers the neighbour it opens over |
| 4. Fields on the card, edit mode only | 1048 × 670 | 3.6 | 0 px | all whole |
| 5. Whole-recipe sheet beside a mini map | about 240 × 670 of map | none at full size | 0 within the sheet | the canvas is gone |
| 6. Drawer, run detail only | 1048 × 335 while open | 3.6 | n/a: no settings | as today |
| **A. 1 as a leaf under the card, plus 6 (mock A)** | **1048 × 670 before a run** | **3.6** | **about 180 px, straight down, same x** | **all whole; branches below move down** |

## Options × criteria

| | 0–6 settings, and the template | One skeleton? | Looks | Against the owner's rulings |
|---|---|---|---|---|
| 1. Face becomes the form (C) | 1–3 short settings fit the card's empty lower two-thirds. MP3's five and the template need 576 × 240 | no: the card's size depends on its settings | calm when it fits; a grown card breaks the 288 × 192 rhythm | fine. After a run, the outcome disappears while you edit |
| 2. Inspector (B) | anything, scrolling. The template fits stacked at 320, but the example line is cut | yes | tidy, but generic: every node editor has one | the card shows a value and a panel 500 px away edits it; that's the drawer's problem turned sideways |
| 3. Popover | like A, but floating | yes | a floating card over cards; shadows pile up | **the one-setting popover was cut** (`cards.md`, *Cut*). It covers neighbours |
| 4. Fields on the card | two 176 px fields at most; the template and MP3 don't fit | no | the best for one setting | **clashes:** settings can be changed outside editing, and an unset folder asks before Start. Edit-mode-only hides both |
| 5. Recipe sheet | unlimited | yes | a form, not a recipe | good for checking every setting at once; bad as the only place. It's a second view of the same values |
| 6. Drawer, run only | n/a | yes: one full-width list | the best the drawer has looked: no rail, no empty half | none. It removes the master/detail split |
| **A. Leaf + run-only drawer** | 1 setting: 64 px. 5 (MP3): 208 px. A template, a drive list or Measure: a two-card leaf (576 × 208) | **yes:** label at 16, control at 96, rows of 28 every 8 | the card and its leaf read as one object, with one outline | settings stay on the card. No links, no toggles, no drag |

## Recommendation: A, the settings fold out under the card; the drawer keeps the run

**The leaf.**
- **Where:** directly under the card, flush, the card's own width (288), on the card's own grid: labels at x 16, controls at x 96, rows of 28 with 8 between, 16 margins. Its fill is `surface`, one step back from the card's `raised`, and a single action-blue outline goes around card and leaf. See frames A1 and A2.
- **Two cards wide (576) when a control needs the width:** the name template (preset, field, example, tokens), the Drive list with Offset and *Measure*, Tell me's ntfy fields. It stays left-aligned to the card, so the labels you read on the card sit directly above the controls that change them. At the recipe's right edge it simply extends, and the camera glides to fit (A3).
- **Space is made, not covered:** branches below the card's branch move down by the leaf's height, in 180 ms. Nothing moves sideways, so the left-to-right order you know never shifts. Wires stay attached.
- **The card is the preview:** the card above shows each change as you make it. Change the level and *Level 8* becomes *Level 5* with the blue dot. Nothing is said twice; the leaf has no title, because the card above it is the title.
- **Past what's on the card:** the leaf shows all of a step's settings, the card's first. MP3 shows Mode, Quality, Stereo, ID3 and ID3v1 on five rows. An *Advanced* row folds rarely used ones, so the leaf is never taller than 208 (13 modules), about one card.
- **Yours, Reset, locked:** a changed value gets the blue dot on the card and in the leaf. *Reset* appears at the leaf's lower right only when there's something to reset. A running step has no leaf: its values are locked on the card with a lock, and there's nothing to edit.
- **Value questions are settings,** so they open the leaf, with the field outlined amber: *Choose a folder*, an unknown offset with *Measure*. The card's amber outline and the leaf are one shape.
- **No settings** (Audio CD, the lookups, Choose, Hold): no leaf. A click opens the run, if there is one, or only selects the card.
- **In edit mode** every card gets a leaf, ending in a quiet *Remove* at its lower right. A card with no settings gets a one-row leaf holding only *Remove*.
- **Leaf and drawer never stack.** After a run, card and Library leaf need about 400 px and the open drawer leaves 335, so unfolding a leaf shuts the drawer, and opening the run folds the leaf. There's one surface below the card at a time.
- **Next disc's loop** enters Drive from below, where Drive's two-card leaf goes. The loop drops by the leaf's height, like any branch below; the mock leaves the loop out.
- **Not the popover that was cut.** That one opened from a single link-styled value and floated over the canvas. The leaf opens from the card itself, holds all of the step's settings, is joined to the card by one outline, and makes room instead of covering.

**The drawer: run detail only.** Full width, no rail:
- header: pill, sentence, actions;
- tabs: Items, Found, Log;
- 24 px rows.

It opens when a step needs a pick (Choose, Rip's retries), and when you open a run. At half the canvas it shows ten rows of the rip with nothing beside them (A4). Rip's *First read* isn't repeated there; the card shows it, locked.

**One click, two places, by position.**
- A card's top third already holds its name and settings, so a click there unfolds the leaf.
- A click on the outcome, detail or item strip opens the run in the drawer.
- Before a run there's no run, so a click anywhere unfolds the leaf.
- Hovering the top third tints it `surface`. There's no underline or chip: it doesn't look like a link, and the owner ruled links out.
- **For the quorum:** is a split card too subtle? The fallback: a click opens whichever applies (leaf before a run, drawer after), and the leaf is kept for a second click on the settings rows.

**Runner-up: B, the inspector.** It's the safest build (one panel, scrolls, any count). It costs 1.2 cards of canvas for good and puts the control 350–790 px from the value. Pick it if the leaf's push feels restless on real recipes.

**C, the card as the form** (option 1) is the most beautiful at one or two settings (C1, C2). But it changes the card's size per step (C3: 576 × 240), and after a run it hides the outcome while you edit. Keep it as the leaf's ideal; don't make it the rule.

**Keep 5 as a second view, not the editor.** A *Settings* view of the whole recipe (a form beside a mini map) answers "what is this recipe set to?" at once. It edits the same values and replaces nothing.

## The general rule, for every future drawer

A surface splits into master and detail only when **both halves are read together**. Choose qualifies: a candidate beside the disc's tracks. A surface never splits to put **two jobs** side by side, such as setting up a step and watching it run. Before adding a column to any drawer, ask: will the eye go back and forth between the halves in one task? If not, it's two surfaces.

## Open for the other lenses

1. Interaction: the click on the top third versus a click anywhere (above), and keyboard: Tab into the leaf, Esc to fold it.
2. Visual: whether the leaf's `surface` fill or a hairline at the fold reads better in dark mode (A2 dark is fine, but the contrast is low).
3. Layout engine: the push needs the layout to accept "this card is 192 + h tall" for one node. That's a single extra input to the tree layout, not a new mode.
