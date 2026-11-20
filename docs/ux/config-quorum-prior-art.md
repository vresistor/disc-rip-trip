# Where a step's settings go: prior art

One of three quorum papers on the owner's question: *is the drawer the best place for a step's settings?* This one looks at how other node and pipeline tools let you set up a node. Read with [cards.md](cards.md) and `DESIGN-UX.md` *The drawer*.

## What the shots say first

| Evidence | Where | So |
|---|---|---|
| Settings lock while a step runs; the card shows the lock | `running-light.png`, `drawer-rip-wide-light.png` | settings and the run view are almost never needed at the same moment |
| A running FLAC drawer's rail holds one folded *Advanced*, and nothing else | `drawer-encoder-progress-light.png` | the master/detail split costs 192 px to show nothing |
| An idle MP3 drawer spends half the window on five controls | `drawer-mp3-light.png` | an idle drawer is far too big for 0–6 settings |
| An idle Drive drawer: two radio rows and an offset field, across 1,040 px | `drawer-drive-light.png` | the same |
| `cards.md` *Rails* already drops the rail for all but about ten steps | `cards.md` | the split is mostly gone already; what's left is the leftovers |
| Choose's question uses the drawer's whole width well | `drawer-choose-light.png` | the drawer is good at *run* content: lists, questions, retries |

**In short:** the drawer joins two things that are needed at different times. Config is small, wanted before a run, and locked during it. Run detail is big, wanted during and after a run, and has nothing to set.

## Prior art

| Tool | Where a node's settings live | Source | What worked, and what changed |
|---|---|---|---|
| Blender nodes | a compact set drawn **on the node**; a fuller set in the **sidebar (N)** for the selected node | API: `draw_buttons` "Draw node buttons", `draw_buttons_ext` "Draw node buttons in the sidebar" ([bpy.types.Node](https://docs.blender.org/api/current/bpy.types.Node.html)) | two tiers, the same as our card (2 settings) plus somewhere for the rest. The sidebar is docked and stays shut most of the time |
| ComfyUI | **widgets on the node**; every value editable in place | [frontend #5692](https://github.com/Comfy-Org/ComfyUI_frontend/issues/5692): widgets and input sockets "co-exist"; a connected widget becomes read-only | needs nothing else, but nodes grow big and busy. Their fix for "can't edit now" is read-only in place, like our lock |
| Node-RED | an **edit tray** that slides over the canvas on double-click, with OK and Cancel | [edit dialog docs](https://nodered.org/docs/creating-nodes/edit-dialog); [dock request](https://discourse.nodered.org/t/editor-dock-the-node-properties-panel/11532) | users asked for a docked panel because clicking node to node is slow. Maintainers refused: it takes space from flows, and "a more assertive action to show the edit dialog" is better |
| n8n | the **node details view**: input data, then parameters, then output, over the canvas | [blog](https://blog.n8n.io/reduce-context-switching-with-node-details-view-product-update); layout from use | great when settings depend on the data (mapping fields). It hides the canvas, so it's heavy for a two-value node |
| Unreal Blueprints | a docked **Details panel** for whatever is selected | [Epic docs](https://dev.epicgames.com/documentation/unreal-engine/details-panel-in-the-blueprints-visual-scriting-editor-for-unreal-engine): "normal with many Blueprint nodes … to see a primarily blank Details panel" | a permanent inspector is mostly empty for small nodes, which is our case |
| TouchDesigner | a **parameter dialog**: docked on P, or floating from right-click; several can be open; pages as tabs | [Parameter Dialog](https://derivative.ca/UserGuide/Parameter_Dialog) | built for hundreds of parameters. It has a *hide default parameters* switch: show only what you changed |
| Apple Shortcuts | **fields inside the action card**; tap a value to change it; *Show More* folds the rest | from use, unverified (the [user guide](https://support.apple.com/guide/shortcuts/apdd02c2780c/ios) covers only variables in fields) | the card is the form. Fine in a vertical list, where cards can grow; on a 2-D canvas, growth reflows the layout |
| Home Assistant | inline expanding cards **until 2025.9**, then a **right sidebar** (a bottom sheet on small screens) | [2025.9 notes](https://www.home-assistant.io/blog/2025/09/03/release-20259/): "instead of that item expanding, it will open a new sidebar to the right", to "keep an overview"; [2025.10](https://www.home-assistant.io/blog/2025/10/01/release-202510/) made it resizable | **moved away from expanding in place**; their stated reason is keeping the overview in view while you edit |
| Zapier | a **right sidebar** per step, now resizable and floating, with a *focus mode* for big steps | [help article](https://help.zapier.com/hc/en-us/articles/29508922694797-Setting-up-triggers-and-actions-just-got-better) | the sidebar kept growing (resize, float, *focus mode*): one panel width doesn't suit every step |
| Make | **module settings** open on click, and on add; *Advanced settings* fields hidden behind a switch; an asterisk until OK | [module settings](https://help.make.com/module-settings) | small and local; a floating panel by the module (from use) |
| Figma | a docked **right inspector**, always on; labels optional | [Navigating UI3](https://help.figma.com/hc/articles/23954856027159) | right when you edit all the time. Forum threads complain it's dense and can't stay collapsed |
| Lightroom | **presets**: a whole set of export settings under one name | [export presets](https://helpx.adobe.com/lightroom-classic/help/export-presets-settings-plug-ins.html) | great with dozens of settings; with 0–6 it's another layer |

## The patterns

| Pattern | Examples | Works when | Costs | Fit here (0–6 settings; Names template; Drive list; app keys; *Choose a folder*) |
|---|---|---|---|---|
| **In-node widgets** | ComfyUI, Shortcuts | few values; the node can grow | busy nodes; reflow on a canvas; small hit targets at *Fit* | ✗ as the editor: breaks the 288 × 192 card and its five slots. ✓ as the display (already done: label and value) |
| **Expand in place** | Shortcuts *Show More*, HA before 2025.9 | a vertical list | moves its neighbours; HA dropped it | ✗ on a 2-D canvas |
| **Docked inspector** | Blender N, Unreal, Figma, Zapier, HA now | editing all the time; many settings | permanent width; often blank (Unreal says so) | ~ at 1280: 1280 − 232 − 288 leaves ~760 px, 2.6 cards. Blank for the ~15 steps with no settings. Holds the Names template only if 2 cards wide |
| **Bottom drawer** (today) | HA on small screens, our app | wide content: lists, questions | half the window for any content; master/detail when two kinds share it | ✓ for run detail and questions. ✗ for 0–6 settings |
| **Slide-over tray** | Node-RED | a deliberate edit with OK and Cancel | modal-ish; slow to hop between nodes | ~ the same space problem as the drawer |
| **Full modal** | n8n | settings depend on the data | hides the canvas and the run | ✗ nothing here maps data |
| **Popover (anchored sheet)** | Make (from use, unverified); little verified precedent on canvases | few settings; local; quick | must be drawn at screen scale, kept on screen, and not left floating at *Fit* | ✓ holds every step: 0–6 rows, the Drive list, a key field. The Names template needs a sheet 2 cards wide |
| **Edit mode only** | many builders | settings rarely change | hides config; a mode | ✗ the owner wants settings visible; `cards.md` already rejected C |
| **Presets** | Lightroom, our Names dropdown | many settings move together | another named layer | ~ only where it already exists (Names) or for whole encoders (*Small and good*, *Transparent*); not a headline |

## Recommended

**1. Split by time: settings leave the drawer, and the drawer is the run.** No rail, ever. The drawer holds Items, Found and Log, the question (*Which release?*, retries, *Let through*), and the header's actions, across the full width. It opens on a run, on *Needs you*, or on a click on a card that has a record.

```
┌ Recipes ┬ CDs  The White Stripes  ! Needs you ─────────── Working  Stop ┐
│         │   … canvas, the asking card scrolled into the part showing … │
│         ├══════════════════════════ ─── ═══════════════════════════════┤
│         │ ↓ Rip  ! Needs you  Track 9 doesn't match AccurateRip.        │
│         │                           Keep  Combine reads  [Read again] ✕ │
│         │ Items  Found  Log                Needs you 1  Verified 16  All │
│         │ 9  Broken Bricks              1:51  ✕ Doesn't match  Leave out│
└─────────┴──────────────────────────────────────────────────────────────┘
```

**2. Settings open on the card, as an anchored sheet.** The card shows two settings; its sheet holds all of them (Blender's two tiers). **This wins on fit, not on precedent:** the verified moves (Home Assistant, Zapier, the Node-RED request) all went to a docked sidebar. The sheet suits 0–6 settings, leaves the canvas whole at 1280, and avoids Unreal's blank panel. It is also Node-RED's "more assertive action".
- **Shape:** 288 wide, one card width (576 for Library's Names template). The card's own two rows come first, at the same x 16 / x 96, now as controls; then the rest, then *Advanced* folded.
- **Placement:** below the card (above it if there's no room), drawn at 100 % whatever the zoom; the camera glides the card into view first.
- **Behaviour:** changes apply at once, as settings outside editing already do (no OK and Cancel; Node-RED's worry doesn't arise). In edit mode they join the draft and undo.
- **What else it holds:** the dot for *yours*, with the date and *Reset* on hover; *Remove* at its foot in edit mode; *Measure* for the Drive.
- **Setup questions:** an unset value (*Choose a folder*, an unknown offset) opens the sheet with that field focused; the card is amber.

```
   ┌ ♪ MP3 ────────────────── ○ ┐   ┌ ▥ Library ───────────────── ○ ┐
   │ Quality   V0               │   │ Folder  ~/Music/MP3             │
   │ Stereo    joint            │   │ Names   Artist / Album / 01 - … │
   │                            │   └─────────────────────────────────┘
   └────────────────────────────┘
   ┌────────────────────────────┐  ← 288, shadow, over the strip below
   │ Quality   [ V0         ▾ ] │
   │ Stereo    [Joint | Full ]  │
   │ Mode      [VBR|CBR|ABR]    │
   │ ▸ Tags and art             │
   │                   Remove   │  ← edit mode only
   └────────────────────────────┘
```

- **Opening it reopens a ruling.** `cards.md` cut "the one-setting popover" and settings "as links". This sheet is different: it's the whole step's settings, opened from one place, and the values still read as facts.
- **One click, one meaning, by where you click.** Each recipe keeps its last run, so after a first run every card has a record. A rule by state would mean the drawer always wins. Instead:
  - the **top third** (name and settings, y 16–64) opens the sheet, with a hover fill and no link styling;
  - the **rest** of the card opens the drawer.
- **Steps with no settings** have no sheet; their top third opens nothing, except in edit mode, where the sheet holds just *Remove*.
- **Key steps** (TMDb, Discogs, GnuDB) show nothing on the card today. They would need *Key* *set* or *Key* *needed* in the settings slot, so the key has a target. That is a change to `cards.md`.
- **The owner decides** whether top third versus the rest is clear enough at a glance.

**3. The alternative to weigh against 2: a docked inspector, only while a card is selected and idle.** Home Assistant and Zapier both went this way. It keeps one fixed place, and it reads well at 1920. It's 288 wide on the right, and shut while running, because settings are locked then. Its cost is width at 1280 and a blank panel for the ~15 steps with no settings; it's better than the sheet only if people set many steps in a row.

```
┌ Recipes ┬ canvas (~760 at 1280) ──────────────────┬ ♪ MP3 ──────── 288 ┐
│         │  [Rip] ── [MP3]* ── [Library]           │ Quality [V0     ▾] │
│         │                                         │ Stereo [Joint|Full]│
│         │                                         │ ▸ Tags and art     │
└─────────┴─────────────────────────────────────────┴────────────────────┘
```

**Not recommended:** a full modal (n8n), edit-mode-only, expanding the card in place (Home Assistant left it), and presets as the main way in.

**Values that belong to a thing** still show in their step's sheet, as today, and one change applies everywhere: keys are *app-wide*; a drive's offset is *this drive's*.

## If config leaves the drawer

The drawer holds only what a run makes: the step's state and sentence, its question (Choose's options fill the body) or retries as header actions, and Items, Found and Log across the full width, with no rail.
