# Cards and drawers: what each one says

What every tile and drawer shows, step by step, and how it looks. Audit: [cards-audit.md](cards-audit.md). Mockups: [cards-mockups.html](cards-mockups.html).

**The rule.** A line is on a tile only if it tells you something about this run, or about this tile's own settings, that you'd act on or want to know. A description of what the step always does belongs in the + picker, never on the tile.

Three more rules follow from it:
- **A step knows only its wires.** Its tile, drawer and record show what came in on its input, its own settings, and what it gave.
- **Nothing is said twice.** The tile is a glance; the pill is the state; the sentence is the outcome or what's needed; the tabs are the detail.
- **One card shape, chosen by composition.** Every tile is 288 × 192: a 3:2 card on a 16 px module, with room to show its whole content. What changes from today:
  - the subtitle's place shows the step's settings, as label and value;
  - no repeats, and the state is shown once, by the glyph and the outline;
  - no filler;
  - the hierarchy below;
  - the same five slots on every card, ending in one item strip.

## Visual hierarchy

After Tailwind's scale (a 4 px unit; type paired with line heights 12/16, 14/20, 16/24) and *Refactoring UI*: emphasise by de-emphasising, a few greys, colour for meaning.

**1. One first thing per block.**

| Block | First | Second | Last |
|---|---|---|---|
| Tile | the outcome (*16/17 verified*) | the step's name, and the detail | the settings, and the item strip |
| Drawer header | the pill (the state) | the sentence (what it means, what's needed) | the step's name, and the actions |
| Row | the item's name | its verdict | number, length, size |

**2. Emphasis by weight and shade, not size.** Every piece of text has a role, and the role sets its size, weight and shade. There are no one-offs.

| Role | Size / line | Weight | Shade | Used for |
|---|---|---|---|---|
| **Outcome** | 20 / 24 | 600 | ink | a tile's result |
| **Title** | 16 / 24 | 600 | ink | a drawer's title |
| **Body** | 13 / 16 | 400 | ink | a tile's detail while working; settings values; the drawer sentence, row names, rail values |
| **Detail** | 13 / 16 | 400 | secondary | a tile's detail once done |
| **Label** | 13 / 16 | 500 | secondary | a tile's name, tabs, rail labels, pills, filters |
| **Meta** | 13 / 16 | 400 | tertiary | settings labels, a qualifier, row numbers, lengths, sizes (tabular figures) |

- **Sizes:** three, from one ratio of 1.25 (a major third): **13 · 16 · 20**. 13 × 1.25 = 16.25 and × 1.25² = 20.3. A tile uses only 13 and 20, so the outcome is the one big thing on it: 1.56 times the rest. The drawer uses 13 and 16.
- **Shades:** three. *Ink* `#18202b`, *secondary* `#4c5563`, *tertiary* `#6b7480`, from `theme.rs`, which also has the dark values.
- **A tile's name is a label.** It sits quieter than the outcome: secondary, at 500. It doesn't shrink; the icon and its position already say which step it is.

**3. De-emphasise; don't shrink.** The name, the settings, the details and a qualifier are all 13 px; they differ only in weight and shade. On the settings line, a default value is tertiary; one you changed is ink, with a dot before it.

**4. Spacing does the grouping.** A tile has three bands, one per third (see *Composition*):
- the head (name, settings);
- the outcome;
- the detail (what it's on, or the one fact about the result), then the item strip.

There are no dividers; the thirds and the 16 px margins do the grouping.

**5. Colour carries meaning only.** It's used for:
- the state glyph, the outline and the pill;
- the progress bar;
- a verdict's glyph;
- the step-kind icon;
- the action blue.

Text is never coloured by state, and done tiles aren't tinted.

**6. Alignment.** Every element sits on the card's grid (see *Composition*), on one left edge, 16 in. A row that's absent leaves its place empty, so the outcomes, details and pictures of a whole strip line up.

**7. Nothing short is cut.**
- **Never cut:** names, outcomes, qualifiers and settings values. That includes *level 8*, *V0 · joint stereo*, *~/Music/FLAC*, *Artist / Album / 01 - Title*, *The White Stripes · 1999*, *Sympathy · SFTRI 577* and *Track 6 · Wasting My Time*. The card and the type are sized so every built-in step's realistic contents fit whole.
- **Two lines before cutting:** the settings and the outcome may each wrap to a second line, which the grid allows. A value never breaks inside itself.
- **What may shorten,** with the full text in a tooltip, is only text with no bound:
  - a long track or album title keeps its start (*The Rise and Fall of Ziggy Stardust and the Spi…*);
  - a deep path keeps its ends (*/mnt/nas/…/FLAC*).

## Composition

**The card: 288 × 192, 3:2,** on a 16 px module, 18 × 12.
- 3:2 against √2 or 4:3: thirds of 3:2 on a 16 px module land on whole modules (x = 96, 192; y = 64, 128). Every line is on the grid, not just near it.
- **Margins:** 16 on every side, one module.

| Band | y | Holds |
|---|---|---|
| **Top third** (16–64) | name 16–32 (the glyph centred on the name's cap height), settings 32–64: two rows, label at x 16, value at x 96 (the first third line) | identity and how it's set |
| **Middle third** (64–128) | the outcome, bottom-aligned to the **lower third line (128)**, up to two lines | **the focal point** |
| **Bottom third** (128–176) | the detail, 136–152; the item strip, 168–176 | what it's on, or the one fact about the result; how far each item has got |

- **Cover art** puts the cover in the lower-left cell, 96 × 96 (x 16–112, y 80–176): six modules, the width of a third less a margin. Its text starts at x 128.
- **The item strip** is always 8 high on the bottom margin, from x 16 to 272 (x 128 on Cover art).

**The eye path**, across a canvas:
1. **The outline:** only on cards working, needing you or failed. Done and waiting cards have none.
2. **The outcome:** 20 / 600 / ink, the only large text, on the lower third line.
3. **The name:** 13 / 500 / secondary, top left.
4. Everything else, at 13 in secondary or tertiary.

To keep that order:
- the step's icon is 14 px with a 1.6 stroke;
- the state glyph is 14 px;
- the label badge is plain text;
- settings labels are tertiary; values are ink at 400, quieter than the outcome's 20 / 600.

**The canvas: one rhythm from the card.**
- **Gap between strips:** 64, a third of the card's height.
- **Gap between stacked branches:** 32, a sixth.
- **Wires** bend with their control points half the gap out (32).
- **Next disc's loop** turns with a 16 radius.

The CD recipe is 2,784 × 640 at 100 %, so a 1280 × 720 window shows about three and a half cards across. *Fit* takes it to 35 %. That's the cost of cards that hold their whole content; *Fit* and following the work (`quorum-plan.md`, asked 1) matter more now.

**The drawer: 3:2 with the canvas.** By default the drawer takes 2/5 of the space under the header and the canvas keeps 3/5: 268 px in a 720 px window, 412 in 1080. That's enough for 7 rows (13 at 1080), and the card being worked on stays whole above it. Its rail is 192 wide, two-thirds of a card (12 modules), so a step's settings column is in proportion with the card it belongs to.

In the mockups, *Show grid* draws the thirds and margins on every card, and the gaps on the recipe.

### Spacing

One scale: **4, 8, 12, 16, 24**. Every padding and gap comes from it, and contents sit centred in their band.

| Element | Value |
|---|---|
| Tile | 288 × 192; 16 margins; the bands in *Composition* |
| Settings | two rows of 16; labels at x 16, values at x 96; a dot (yours) or lock 4 before a value |
| Canvas | 64 between strips, 32 between stacked branches |
| Drawer chip | 20 high; 8 at the sides; 4 apart |
| Pill | 20 high; 8 at the sides; glyph 4 before the text |
| Drawer | 2/5 of the space under the header (268 at 720, 412 at 1080) |
| Drawer grip | 12 = 4 + 4 handle + 4 |
| Drawer header | 48 high, items centred; 16 at the sides; 8 between title and pill, 12 before the sentence, 8 between actions |
| Buttons | 28 high; 12 at the sides |
| Tabs row | 32 high; 16 between tabs; filters 4 apart |
| Work column | 16 at the sides |
| Rows | 24 = 4 + 16 + 4; 12 between cells |
| A row that needs you | 48 = 8 + 16 + 16 + 8 |
| Rail | 192 wide; 16 at the sides; rows 28 = 4 + 20 + 4 |
| Popover | 12 inset; 8 between rows |

### Before and after, element by element

| Element | Today (size / weight / shade) | Proposed role |
|---|---|---|
| Tile name | 13.5 / 600 / ink | **Label** 13 / 500 / secondary: identity, not the news |
| Tile subtitle (*lookup*, *cyanrip, AccurateRip*) | 12 / 400 / secondary | **gone**: a description of the step, not the run |
| Settings | the subtitle sometimes held one (*level 8, verified*) | label and value in the subtitle's place: label **Meta**, value **Body**; not a link |
| Status line | 14–15 / 500–700, coloured by state | **Outcome** 20 / 600 / ink, always; the glyph and outline give the state |
| Detail line | 12 / 400 / secondary; a qualifier or what's happening | split. A qualifier is **Meta** on the outcome's line. The detail is one line: **Body** while working, **Detail** when done |
| Done tint | green wash | none; the green ✓ says it |
| Progress bar | on some tiles | the item strip, on every card (see *The item strip*) |
| Drawer title | 17 / 700 / ink | **Title** 16 / 600 / ink |
| Pill | 12.5 / 600 / state colour | **Small** 12 / 500, state colour (meaning) |
| Sentence | 13.5 / 400 / secondary | **Body** 13 / ink: it's what you need to know |
| Tabs | 13.5 / 500 secondary, 600 ink when on | **Label**; on: ink at 600 with the 2 px underline |
| Captions, In/Out line | 12–12.5 / secondary or tertiary | gone |
| Row name | 13 / 400 / ink | **Body** |
| Row number, length, size | 12 tertiary, 12 secondary, 12 mono | **Meta** 13, tabular figures |
| Verdict | 12.5 / state colour, words and glyph | glyph in the state colour, word **Label** secondary |
| Rail label · value | 12.5 secondary above 13 ink | **Label** · **Body**, on one line |
| Provenance (*yours, 19 Nov*) | 11.5 / tertiary | the blue dot; the date on hover |

## Tiles

### Shape

Every tile is **288 × 192** (see *Composition*). Today's is 152 × 112.

**What the size holds:** five slots, the same on every card (next section). Every built-in step's realistic contents fit whole: *Colony Wars (Europe) (Disc 1)*, *~/Music/FLAC*, *Artist / Album / 01 - Title*, *Track 6 · Wasting My Time*.

**State:**
- The glyph, plus a 2 px outline in the state's colour while working, needing you or failed.
- No tint, and no coloured text.
- Before a run, only the name and the settings show.

### Slots

Every card has the same five slots, each with one meaning on every card. A slot with nothing for it is empty, and its space is kept.

| Slot | Where | Means, on every card |
|---|---|---|
| **1. Name and glyph** | 16–32 | which step; its state |
| **2. Settings** | 32–64 | the step's own settings, up to two, as label and value; empty if it has none |
| **3. Outcome** | ends on 128 | the result in the job's unit, plus at most one qualifier |
| **4. Detail** | 136–152, one line | while working or needing you: what it's on right now (*Track 6 · Wasting My Time*, *Track 9 doesn't match AccurateRip*). When done: the single most useful fact about the result (*SFTRI 577 · Sympathy for the Record Industry*). Never two lines |
| **5. Item strip** | 168–176 | one cell per item the step handled; the same visual on every card, and the only progress bar |

**Cover art** is the one exception: its cover fills the lower-left cell (96 × 96), and slots 3 to 5 start at x 128.

### The item strip

- **Which items:**
  - tracks for Audio CD, Rip, the encoders, ReplayGain, Lyrics, Folder and Hold;
  - titles for the video steps;
  - files for CHD, Keep, Cue, Lyrics file and M3U.
- **A step that handles the disc as a whole** shows a single cell: Drive, the lookups, Choose, Check, Image, Notify, Next disc.
- **Equal cells,** 2 apart. Track lengths would only fit the track steps, and the strip must look the same everywhere. The disc's shape, the sector map and the gain spread go to the drawers.
- **Before the items are known** (before a run), a single waiting cell.

**One legend, everywhere:**

| Cell | Means |
|---|---|
| light fill | waiting |
| blue | working on it |
| dark grey | done |
| green | verified; only on a step that verifies (Rip, Check) |
| orange | needs you |
| red | failed |
| outlined, no fill | left out (skipped, a title not kept, a track with no lyrics) |

### Settings on the card: three ways

The mockups compare three treatments on FLAC, MP3, Rip, Library and Transcode. Each is shown idle, changed by you, and locked while ripping, and the slot positions are the same in all three.

| | A. Label and value | B. Quiet tags | C. Edit mode only |
|---|---|---|---|
| Looks | *Level* **8**, *Folder* **~/Music/FLAC**: quiet labels at x 16, values in ink at x 96 | small borderless tags on a light fill: *level 8*, *V0*, *joint stereo* | no settings on the card; in edit mode, as A |
| Click | the card opens the drawer, where settings are edited | each tag opens its one-setting editor | in edit mode, as A |
| Yours | a blue dot before the value | a blue dot in the tag | as A |
| Locked | a lock at the end of each label, so values stay on x 96 | a lock in each tag | settings hidden anyway |
| Fits the slot | two rows of 16 | two rows only at 14 px tags, small to click | as A |

**Picked: A.**
- It reads as facts, not links, so it promises nothing it doesn't do.
- The label says what a bare *8* or *slow* means.
- Its values sit on the first third line, so it's part of the grid.

C would hide what `DESIGN-UX.md` puts on the tile on purpose, and B brings back the chips you moved away from. With A, the single-setting popover goes too; one click on the card opens its drawer.

### Step by step

**Columns:**
- **Settings:** label and value; blank where the step has none.
- **Outcome:** done, and so far while working.
- **Detail, working or needs you:** what it's on right now.
- **Detail, done:** the one most useful fact about the result.
- **Strip:** what each cell is.

| Step | Settings | Outcome | Detail, working or needs you | Detail, done | Strip |
|---|---|---|---|---|---|
| Drive | Drive *Blu-ray drive*; Offset *+103* | *CD in* / *Empty* / *Not connected* | *Reading at 8×*; *Insert a disc*; *Offset unknown* | | the disc |
| Audio CD | | *17 tracks* · 43:45 | *Reading the table of contents* | | tracks |
| Data disc | | *ENCARTA98* | | *ISO 9660 · 682 MB* | the disc |
| Game disc | | *SLES-00860* · PlayStation | | *Europe* | the disc |
| Video disc | Shortest *60 s* | *1 of 3 titles* | *Listing titles* | *Title 1 · 1:57:30* | titles; left out outlined |
| MusicBrainz | | *2 releases*; · by layout, when not by disc ID | *Asking by disc ID* | | the disc |
| CD-Text | | *17 titles* / *None* | | *THE WHITE STRIPES* | the disc |
| Discogs, GnuDB | | *1 release* / *No answer* | *Asking by barcode*; *Needs a token* | *no answer after 10 s* (skipped) | the disc |
| Cover art | Largest *1200 px* | *1200 × 1200*, the cover beside it | *Fetching the front* | | the disc |
| Lyrics | | *14/17 synced* | *Track 6 · Wasting My Time* | *none for 3, 10, 15* | tracks; none outlined |
| TMDb | | *3 films* · "blade runner" | *Searching "blade runner"* | | the disc |
| Redump | | *1 entry*; · by serial only, when not by hash | *Looking up PlayStation* | | the dump |
| Choose | | *The White Stripes* · 1999 | *Which release? 3 fit* | *SFTRI 577 · Sympathy for the Record Industry*: the short catalogue number first, so only the label name can shorten | the disc |
| Rip | First read *fast* | *16/17 verified* · 1 kept | *Track 6 · Wasting My Time*; *Track 9 doesn't match AccurateRip* | *track 9 kept unverified*, when one was | tracks: verified, reading, to retry |
| Image, game Rip | Retries *20* | *682 MB* | *Reading · 3 min left*; *3 sectors unreadable* | *3 sectors refined*, when any were | the dump |
| Rip (video) | | *1 title* | *Blade Runner · 12 min left* | *Blade Runner · 7.6 GB* | titles |
| Check | Catalogues *PlayStation* | *Matches Redump* / *Not in Redump* | *Hashing 2 files* | *Encarta 98 Encyclopedia (USA) (Disc 1)* | the dump; verified green |
| Hold | | *4 held* / *17 let through* | *All 17 in Review* + **Let through** | | tracks |
| FLAC, WavPack | Level *8*; Compression *high* | *294 MB* · 58 % | *Track 6 · Wasting My Time* | | tracks |
| ALAC, WAV | | *412 MB* | as FLAC | | tracks |
| MP3 | Quality *V0*; Stereo *joint* | *79 MB* | as FLAC | | tracks |
| Opus, AAC, Vorbis | Bitrate *160 kbps VBR* | *53 MB* | as FLAC | | tracks |
| ReplayGain | Gain *track + album* | *−7.3 dB* | *Measuring the album* | *tracks −9.1 to −5.2 dB* | tracks |
| Cue sheet | | *disc.cue, rip.log* | | | files |
| Lyrics file | | *14 .lrc files* | | | files |
| CHD | Codecs *CD* | *412 MB* · 60 % | *Compressing*, *Verifying* | | the file |
| Keep as is | | *682 MB* | | | the files |
| LibCrypt | | *SBI written* / *Not protected* / *Not needed* | | | the image |
| M3U | | *Colony Wars.m3u* · 2 discs / *Disc 1 of 2* (waiting) | | | the discs |
| Transcode | Video *H.265 CRF 22*; Preset *slow* | *2.1 GB* · 28 % | *Blade Runner · 1 h 10 min left* | | titles |
| Folder | Folder *~/Music/FLAC*; Names *Artist / Album / 01 - Title* | *17 filed* | *04 - Suzy Lee.flac*; *Choose a folder* | *The White Stripes / The White Stripes* | tracks |
| Notify | When *always*; Where *desktop + ntfy* | *Told you* / *Didn't tell you* | | *17 tracks filed, all verified* | the disc |
| Next disc | | *Ejected* | *Ejecting* | | the disc |

**Ruled:** Rip always checks AccurateRip; Image and game Rip always refine; Lyrics writes no *.lrc* itself (a *Lyrics file* step after the Library does). **Still pending your yes:** *Catalogues* on Check, shown as it is today.

**Cue sheet:** it has no settings now. It sits after the Library, and names the files the Library made.

**Wires fixes:**
- **Drive:** it no longer names the album.
- **Choose:** its cards no longer show a cover, because Cover art runs after it.

**A setting exists only for a real choice about the step's own output.** A step uses whatever arrived (a cover is embedded if one came) and always does its job properly.

**Kept qualifiers, and why each changes what you think:**

| Qualifier | Why it stays |
|---|---|
| 43:45 | a short total means a single or a partial disc |
| 1 kept | not everything is verified |
| 58 %, 60 %, 28 % | whether the settings packed well |
| PlayStation | the system decides which catalogues apply; a wrong one is worth catching |
| "blade runner" | the query is guessed from the label; a bad guess explains bad matches |
| 1999 | it tells pressings apart |
| 2 discs | the set is complete |
| *by layout*, *by serial only*, *3 sectors refined* | shown only when the result is weaker than usual |

### Cut, and why

| Line | Where | Why it went |
|---|---|---|
| *lookup*, *from the disc*, *by disc ID*, *by barcode*, *by freedb ID*, *by disc label*, *by hash, else serial* | lookups' subtitles | describes what the step always does: the + picker's job |
| *table of contents*, *volume label*, *system and serial*, *MakeMKV*, *eject, start again* | kind steps, video Rip, Next disc | the same |
| *Cover Art Archive*, *PAL PlayStation*, *with its log*, *multi-disc games*, *after Review* | Cover art, LibCrypt, Keep, M3U, Hold | the same |
| *MusicBrainz, CD-Text* on Choose | subtitle | the wires show it |
| *exact* | MusicBrainz | the normal case; only the weaker *by layout* is worth saying |
| *with album* | CD-Text | doesn't change what you'd do with it |
| *85 %* | Discogs | Choose weighs scores; you don't |
| *clean* | Image | the normal case; *refined* says otherwise |
| *by hash* | Redump | the normal case |
| *IBM PC* | Check | already known from the disc |
| *DVD* | Video disc | doesn't change the result |
| *243 KB* | Cover art | the dimensions matter; the bytes don't |
| *7.6 GB* | video Rip | Transcode's ratio uses it; on its own it's trivia |
| *245 kbps avg* | lossy encoders | it restates the chip |
| *album* | ReplayGain | the chip says which gain |
| *3 files* | Keep as is | trivia |
| *294 MB* | Folder | the encoder before it says it |
| *desktop* | Notify | the chip says where it tells |
| *you picked* | Choose | the drawer's *why*; you know whether you picked |
| *embed* chip | Lyrics | it wasn't a setting. The real one, *.lrc too*, shows as [+ .lrc] only when on |
| *Verify* (FLAC, CHD), *Embed the cover*, *Cover size*, *Tags* (encoders) | chips and rails | **the owner's ruling:** a step uses what arrived and always does its job properly. These were never a real choice about the output |
| *AccurateRip* on/off (Rip), *Refine* (Image), *Catalogues* (Check) | chips | the same test. Rip always checks AccurateRip, and the verdict says when the disc isn't in it. Image always refines. Check looks in the system the disc says it is. **Ruled** for the first two; *Catalogues* still awaits your yes |
| the footer band (v3, v4) | every tile | you preferred today's shape; the settings went back to the subtitle's place |
| per-step pictures, a second detail line, other progress bars (v5, v6) | every tile | you wanted every card the same: one item strip, one detail line. The disc's shape, the sector map and the gain spread move to the drawers |
| settings as links, and the one-setting popover | every tile | not a link; the card opens its drawer |
| state colour on text, done tint | every tile | the glyph and outline say it |

## Drawers

### Chrome

| Part | Height | Today | Rule |
|---|---|---|---|
| Grip | 12 | 16 | |
| Header | 48 | 50 | name, pill, sentence, actions, ✕ on one row |
| Tabs and filters | 32 | 28 + 10 + 32 | tabs left, filters right; filters only when two or more groups are non-empty |
| Captions, In/Out line | 0 | 22 each | none; the tab names the content |
| Column headers | 0 or 24 | 22 | only where columns would be ambiguous |
| Rows | **24** | 34 (60 needing you) | one line; a row that needs you is 48 with its reason |
| Rail rows | **28** | ~64 | label left, control right; *yours* is the blue dot |

The first row starts 96 px down, against about 150 today: 9 rows at the default 330 px, against 5. In the drawer cases, every line passes the rule too. The lines cut from the drawers: the In/Out line, the captions, *Uses:* rail facts, *Asked with:*, the zero-count filters, and the offset and drive repeated on Rip.

### Rails

| Rail goes, because everything is on the tile | Rail stays (200 px), with what's left |
|---|---|
| Rip, Folder, Check, ReplayGain, Lyrics, Video disc, Cover art, FLAC, ALAC, WAV, WavPack, AAC, Vorbis | Drive: the drive list and *Measure* |
| No settings at all: Audio CD, Data disc, Game disc, MusicBrainz, CD-Text, Redump, Choose ×3, video Rip, Hold, Keep, LibCrypt, M3U, Next disc | MP3: ID3 version, ID3v1 too. Opus: complexity, tuned for |
| | Image and game Rip: read speed |
| | CHD: hunk size |
| | Transcode: 10-bit, tracks to keep |
| | Notify: ntfy topic and server |
| | Discogs, GnuDB, TMDb: the app-wide key |

### Header sentence and first tab

**Pill:** one word: *Ready*, *Waiting*, the working verb (*Reading*, *Asking*, *Encoding*, *Filing*), *Needs you*, *Done*, *Skipped*, *Failed*. Orange only for *Needs you*.

| Step | Sentence | First tab |
|---|---|---|
| Drive | unplugged: "Plug it in, or pick another." No offset: "AccurateRip doesn't know this drive." | Log |
| Audio CD | the disc ID | **Items**: track, m:ss, ISRC |
| Data disc, Game disc | none | Found, one row |
| Video disc | no MakeMKV: where to get it | **Items**: each title, and why one was left out |
| Lookups | the query or key, or why there's nothing | **Offered**: candidates, best first |
| Cover art | none | Offered: the picture |
| Lyrics | "None for 3, 9, 15." | Offered |
| Choose | asking: what tells the candidates apart ("They differ by year, country and label."). Done: why (*you picked*, *only one*, *best match 90 %*) | asking: one line per option that reached it, with the differences marked, beside the tracks against the disc. Done: Found |
| Rip | one to retry: "Track 9 doesn't match AccurateRip." + the retries as actions. Two or more: *Read all again* | **Items**: those to retry first, then track order |
| Image, Check | failed or no match: the reason + *Keep*, *Leave out* (and *Refine* on Image) | Log / Found |
| Encoders, CHD, Transcode | none, or what failed | **Files**, with sizes |
| Folder | done: the folder it made, as a link | **Files**: planned names greyed before, real ones after; one code path |
| Hold | full: "Listen, then let them through." + *Open Review*, *Let through* | Items |
| Notify | the message in full | none |

**For a decision:** Check already finds Redump's name, so Redump and game Choose add something only when the hash matches nothing. Should they stay in the default games recipe?

## What this supersedes

| Where | Was | Now |
|---|---|---|
| `step-record.md`, *The drawer* | an In/Out flow line | none |
| `DESIGN-UX.md`, *What you see* | a tile is "what it is and what it uses"; status coloured by state | five slots on every card: name, settings, outcome, detail, item strip; state by glyph and outline only |
| `DESIGN-UX.md`, *The window* | "Tiles are always 152 px at 100%" | always 288 × 192 |
| `DESIGN-UX.md`, state table | done tinted green; grey *After …*; red for items to retry | no tint; empty before a run; orange for items to retry |
| `DESIGN-UX.md`, *Configuration is on the tiles* | settings only in the drawer | key settings shown on the card as label and value; edited in the drawer |
| `quorum-plan.md` | done tiles keep a light tint | no tint |
| `heuristic-review-2026-11.md` 4.1, 1.3, 8.1 | "1 to retry" in the pill; grey *After …*; a *default* tag | *Needs you*; empty; a dot only for *yours* |
| `tray.rs` `step_facts` | *Uses:* / *Read from:* rail facts | removed |

## Bugs found on the way

1. A busy step's pill says *Ready* (`state_pill`).
2. Lyrics says "Nothing found." beside "14 of 17"; GnuDB says it after "No answer".
3. TMDb: "3 found 3 films, 0 seasons."
4. Choose's rail shows the engine's lower-cased "after audio cd".
5. Drive rail: the album's name drawn over *Free* or *Busy*, and knowledge Drive shouldn't have.
6. A Folder's Items and Found disagree about the path; a Folder fed a cue and log draws them as tracks.
7. MP3's *v2.3* is shown as *V2.3* (`cap`).
8. M3U says Done while it waits for another disc.
9. The Drive tile says *Empty* while the header says the disc is in it.
10. A Folder's example uses another disc's samples.
