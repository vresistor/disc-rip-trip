# Config quorum: tasks and moments

One of three lenses on "maybe the drawer is not the best way to add config options". This one asks **when, why and how often** each setting is touched. Evidence: the 46 distinct `ParamSpec`s in `crates/steps/src` (49 hits less three `sim.rs` copies), `model.rs` `locked()`, and the shots in `app/shots/`.

## The moments

| Moment | How often | What you're doing | Mode |
|---|---|---|---|
| **Setup** | once per install, once per drive | giving the app what it can't guess: keys, an email, an offset | configure |
| **Shaping** | when you make or tune a recipe; then months of nothing | deciding what the output is: format, quality, folder, names | configure |
| **This disc** | rare; a scratched or odd disc | treating one disc differently | act |
| **Run** | every disc | watching; answering a question; retrying a track | watch, act |

## Every setting by its moment

Scope from the code: **R** recipe, **D** `.on_drive()`, **A** `.app_wide()`. A card shows a setting when it has `.on_card`.

| Card | Keys | Scope | Moment | On card |
|---|---|---|---|---|
| Discogs | `token` | A | setup | no |
| GnuDB | `contact` | A | setup | no |
| TMDb | `api_key` | A | setup | no |
| Tell me | `ntfy_topic`, `ntfy_server` | A | setup | via *Where* |
| Drive | `offset` (`.findable()`, `.only_with(["rip"])`) | D | setup, **only if the lookup fails** | yes |
| Drive | `drive` | R | shaping (a recipe is bound to its drive) | yes |
| FLAC, WavPack, AAC, Vorbis | `level`, `compression`, `bitrate`, `quality` | R | shaping | yes |
| MP3 | `mode`, `quality`, `bitrate`, `stereo`, `id3`, `id3v1` | R | shaping | 2 of 6 |
| Opus | `bitrate`, `mode`, `complexity`, `for` | R | shaping | 1 of 4 |
| every encoder, Transcode | `extra` | R | shaping, experts | no |
| Transcode | `codec`, `crf_h265`, `crf_av1`, `preset_h265`, `preset_av1`, `ten_bit`, `audio`, `audio_langs`, `subtitle_langs` | R | shaping | 2 of 10 |
| Library (each Folder card) | `folder`, `names` | R | shaping; `folder` asked at first run | yes |
| ReplayGain | `mode` | R | shaping | yes |
| Cover art | `size` | R | shaping | yes |
| Video disc | `min_length` | R | shaping | yes |
| Check | `systems` | R | shaping (ruling pending) | yes |
| CHD | `codecs`, `hunk` | R | shaping | 1 of 2 |
| Tell me | `when`, `desktop` | R | shaping | yes |
| Image, game Rip | `retries`, `speed` | R | shaping; **this disc** when scratched | 1 of 2 |
| Rip (audio) | `first_read` | R | **this disc**: a scratched one | yes |
| Rip (audio) | `tracks` (free text, `"all"`) | R | **this disc**, and only knowable after the TOC | no |
| *anything* | | R, D | **run: nothing.** `locked()` freezes the whole recipe while ripping, not only the running step; D also locks while the drive is busy | lock glyph |

**Totals:** 6 setup (5 A, 1 D); 38 shaping, of which `retries` is also this-disc; 2 this-disc only (`first_read`, `tracks`); **0 run**.

## What the table says

1. **Nothing is set during a run, yet every run drawer carries the settings.** `locked-flac`: mid-run, FLAC's drawer is *Locked while ripping · Compression 8 · Advanced*, a whole drawer of dead controls. `drawer-encoder`: the rail is one collapsed *Advanced* beside the track list. The rail is a configure panel that has wandered into a watch view.
2. **Setup items are scattered by step, though they belong to the app.** Five keys sit in four step kinds across recipes. `drawer-api-key`: TMDb's drawer is one field in half a window, labelled *app-wide*: the drawer is admitting it's the wrong home.
3. **The offset belongs to the hardware.** It's per drive, found by itself, and asked only when AccurateRip doesn't know the drive. Every recipe on that drive shows it, but none owns it.
4. **Shaping is rare, deliberate and comparative.** You set FLAC beside MP3 and Review beside Library. Per-card drawers show one card at a time, under a half-hidden canvas.
5. **The this-disc moment is already served by acting on the run.** Rip offers *Read again, Combine reads, Slowly, Leave out* per track. Those are the per-disc answers, given at the moment you know the disc is bad. `tracks` duplicates *Leave out*, as a text field filled before the TOC exists.

## The argument

- **Configuring and watching are different modes.** They happen at different moments, at different rates, with different eyes. Configuring compares and decides once; watching scans and reacts. One drawer serving both is why it splits into a master/detail (rail and tabs) and why neither half is good.
- **Does config belong with run detail?** No. The run drawer should hold only the run.
- **An app Settings page?** Yes, for the six setup items: five keys, plus the offset under *Drives*. They are set once and shared across recipes. The ask stays on the card: *API key?* in the list and *Offset?* on the Drive card each open the page at that field. Setup stays just-in-time; it just has one home.
- **One "recipe settings" view?** Yes, in edit mode: one sheet with every card's settings, in canvas order, **keyed by card, not step kind** (*Review* and *Library* are two sections; so are *FLAC* and *MP3*).
- **What edit mode means: configure.** Everything about a recipe (steps and settings) changes in edit mode, as one draft with undo and Save. This ends *settings changed outside editing are kept at once*: no locks, no half-saved states. Edit is already disabled while ripping (`locked-flac` header).
- **Drive settings: on a Drives page, shown on the card.** The card keeps *Drive · Offset* as label and value (treatment A, unchanged). Which drive is a recipe setting in the sheet; the offset and *Measure* live under Settings → Drives.
- **This disc:** drop `tracks` (*Leave out* covers it). Keep `first_read` in the sheet as the recipe's default; one disc's careful read is *Start ▾ → Read carefully*, used once and not saved.

## Proposals

**1. Edit mode opens the recipe sheet (recommended).** At 1280 × 720 the sheet is 384 wide: 24 modules, four-thirds of a card. The canvas keeps 664, about 2.3 cards. Clicking a card scrolls the sheet to its section. The widest editors fit: *Names* stacks the preset, pattern, preview and tokens in 352; Transcode's `.when` rules leave 7 rows of 28; the drive is a dropdown.

```
┌ Recipes ─┬ CDs  ↶ ↷           Cancel [Save] ┬ Recipe settings ────────┐
│ CDs      │ ┌Drive──────┐┌Audio CD──┐        │ DRIVE                   │
│ Editing  │ │Drive Blu-ray││          │       │ Drive    [Blu-ray ▾]    │
│          │ │Offset +103 ││          │ ...   │ Offset   +103  Drives → │
│ Films    │ └───────────┘└──────────┘        │ RIP                     │
│ API key? │ ┌FLAC───────┐┌Review────┐        │ First read [fast ▾]     │
│          │ │Level ● 6  ││Folder ~/…│        │ FLAC                    │
│          │ └───────────┘└──────────┘        │ Level  ──●──── 6  Reset │
│          │                                   │ REVIEW                  │
│          │                                   │ Folder [~/Music/Rev][▤] │
│          │                                   │ Names  [Artist/Album ▾] │
│          │                                   │  {albumartist}/{album}… │
└──────────┴───────────────────────────────────┴─────────────────────────┘
```

**2. Settings, for what belongs to the app and the hardware.** It opens from the list's foot, beside *Dark*, and holds two sections and nothing else.

```
┌ Settings ──────────────────────────────────────────────────────────────┐
│ Services                         │ Drives                              │
│ Discogs   Token   ••••5678       │ ◉ Blu-ray  +103  AccurateRip  │
│ GnuDB     Email   [          ]   │ ○ DVD      ?     [Measure]    │
│ TMDb      Key     ••••1234       │                                     │
│ ntfy      Topic   [ rips     ]   │                                     │
│           Server  ntfy.sh        │                                     │
└──────────────────────────────────┴─────────────────────────────────────┘
```

**3. The drawer becomes the run, at full width.** No rail anywhere; the per-disc choice moves to Start.

```
┌ CDs  The White Stripes, in Blu-ray drive      Needs you   [Stop] ─────┐
│  …canvas…                                                [Start ▾]    │
├───────────────────────────────────────────────── Read carefully ──────┤
│ Rip  ● Needs you  Track 9 doesn't match AccurateRip.  [Read again] ✕ │
│ Items  Found  Log                                  Needs you · All   │
│ 9  Cannon      2:31  ! to retry   Read again · Combine · Slowly · Leave out │
│ 1  Jimmy the Exploder  2:30  ✓ verified                               │
└───────────────────────────────────────────────────────────────────────┘
```

**Considered and set aside: settings on the card's face in edit mode.** It meets "config visible on the cards" most literally, but 288 × 192 holds 5 rows of 28. Names, Transcode and the drive list would each need a popover, which brings back the one-setting popover `cards.md` removed.

**Costs of 1–3:**
- tweaking FLAC's level is now two clicks (*Edit*, the card), not one;
- the canvas is narrower while editing;
- there is a Settings screen, which the design refused.

The moments say these are cheap: shaping and setup are rare, and the run, which happens every disc, gets a cleaner view.

## What the drawer is for afterwards

- **One job: the run.** It holds the header (pill, sentence, actions), then *Items · Found · Log*, at full width.
- **It opens** when a step needs you, or when you click a card that has a record.
- **Questions stay here:** pick one, a value (*Choose a folder* writes the recipe), a choice per item.
- **The rule for any master/detail in the drawer:** the master must be the run's items, and the detail that item (Choose's candidates beside the tracks; a track beside its log). Settings are never the master.
- **Before a run,** a card has no record, so clicking it enters edit mode at its section.

## Supersedes

| Where | Ruling | Becomes | Why (moment) |
|---|---|---|---|
| `DESIGN-UX.md`, *The idea* | "There is no settings page" | Settings: Services, Drives | setup is once, and shared across recipes |
| `DESIGN-UX.md`, *Configuration is on the tiles* | settings set only in the drawer | edit-mode sheet; values still shown on cards | shaping is rare and comparative |
| same | "First-time setup isn't a screen" | still asked on the card; the ask opens the page | keeps the setup just-in-time |
| `DESIGN-UX.md`, *Editing* | "Settings changed outside editing are kept at once" | settings change only in the draft | one mode, one draft |
| `DESIGN-UX.md`, *Not designed yet* | app things (working folder, MakeMKV key) | Settings → Services | the same moment as the keys |
| `cards.md`, *Rails* | rails kept on Drive, MP3, Opus, Image, CHD, Transcode, Tell me, lookups | no rails | nothing is set during a run |
| `cards.md`, treatment A | kept | kept | config stays visible on cards |
