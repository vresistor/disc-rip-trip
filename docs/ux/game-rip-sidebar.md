# The game rip sidebar, brought together

Rip and Check and fix, for a game or data disc, tell one story: the disc is read, and then the copy is made right. Each sidebar was built up one commit at a time:
- the ring and summary (6009f17);
- passes as rows and the map (6337bf7);
- the Disc/Sectors views (9128920);
- shapes (d7c67ad);
- speed through errors (1cebedf);
- re-read progress and the crawl warning (aa55b9b);
- the hand-on to Check and fix (10f8fa7);
- Check and fix's map, fix, combine and ECC sections and its explanations (038518c, 7f400c5, 1f48c6e, 2d4f226, 7709bbf).

Each piece makes sense on its own. Together they say the same things several times, in different words, in no fixed order.

This note covers four things: an inventory of what's on screen, what's wrong with it, one shape for both sidebars, and what's dropped. The mockup is `ripside-mockup.html`, kept outside the repo. It shows four states at 448 px, the sidebar's width in a 1280 × 720 window, in light and dark.

Sources: the shots in `app/shots` (`rip-game-*`, `games-rip-*`, `step-check-*`, `games-check-*`), `tray.rs` (`dump_rip`, `result`), `compare.rs` and `sectormap.rs`.

## 1. Inventory

**States.** Each state is shown with the shot it comes from:
- **R1** reading clean (`rip-game-reading`)
- **R2** reading through errors (`games-rip-crawling`)
- **R3** re-reading bad parts (`games-rip-rereading`)
- **R4** unreadable, *ask* mode (`games-rip-unreadable`)
- **R5** handed on: no shot. Taken from code: `dump_verdict`'s `made the image with…` branch.
- **R6** done clean, or done after a re-read (`rip-game-clean`, `rip-game-done`)
- **R7** done but wrong (`rip-game-spots`)
- **C1** fixing (`step-check-fixing`)
- **C2** ripping again (`step-check-ripping-again`)
- **C3** verified (`step-check-fixed-verified`)
- **C4** not verified, holding (`step-check-mismatch`, `step-check-repaired-not-verified`)

**Markers.** *Rep* means the element repeats a fact shown elsewhere in the same sidebar. *Both* means the fact appears in both sidebars. *Rare* means it matters only in uncommon cases.

### Rip

| Element | Shown in | What it says | Rep / Both / Rare |
|---|---|---|---|
| Header: icon, *Rip*, pill | all | Reading / Done / Needs you | pill = card status |
| *Open tray* button, a row of its own | R1–R4, after | | costs 40 px; the CD sidebar does the same |
| Tabs Progress · Settings · Log | all | | |
| Ring, centre number | all | 43% read · 3% re-read · *771 unreadable* · *682 MB read / not clean* | **Rep** in R4: the count is also in the headline, a line and the red row |
| Ring arc and position dot | R1–R3 | how far; R3: how far through the *bad sectors*, not the disc | the same ring means two different things |
| *Pass N · Reading* line | R1–R3 | which pass | **Rep**: the row below says *Reading* / *Re-reading* |
| Disc label (*SLES_008.60*) | R1–R3 | | **Rep**: on the Game disc card |
| Progress bar | R1–R3 | | **Rep**: the ring already fills |
| Time left · speed | R1–R3 | *6 min left · 8.0×*, *time left unknown · below 0.1× · slowed by read errors* | |
| *Sector 144,200 of 332,999* | R1–R3 | R3 adds *(55% through the disc) · try 19 of 20* | |
| *30 of 774 bad sectors gone through · 2 recovered so far* | R3 | | |
| Where-line beside the ring | R1, R4, R7 | *Read errors between 54% and 55%*, as a % | **Rep**: given in cm under the map; three units in all (% · cm · sector number) |
| Crawl warning, orange, with its own **Stop** | R2, R3 | *This disc has areas the drive can't read. Cleaning it, or another drive, may help.* | **Rep**: Stop is also in the top bar. **Rare** |
| Headline beside the ring | R4–R7 | *Not read cleanly* / *Read cleanly* / *Bad parts re-read* / *25 sectors came back wrong* | |
| Detail line | R4 | *Couldn't read 771 sectors* | **Rep** (×4 with ring, red row, card) |
| Advice paragraph | R4 | *Clean the disc and choose Re-read bad parts…* | **Rep**: the buttons say the same |
| Size, *Read in 0:01* | R6 | | size **Rep** in the ring |
| Drive name | R4–R7 | *Blu-ray drive* | **Rep**: in the window title |
| Red attention row with buttons | R4 | label, *771 sectors couldn't be read*, *Re-read bad parts*, *Make the image anyway* | **Rep** |
| Pass rows | all | *1 Read the disc · SCSI 455112, C2 211731 · Errors*; *2 Re-read bad parts · SCSI 453348 left · Some left*; *3 Write the tracks · ✓ None skipped / 771 skipped*; *4 Check every sector · ECC 25, EDC 25 · 25 wrong* | counts in the drive's units (samples), not sectors. *Check every sector* clashes with the Check and fix step |
| Sector-order line | rare | *Sector order found by testing…* | **Rare** |
| Heading *Every sector, from the centre out* | when mapped | | |
| Disc / Sectors switch | when mapped | | |
| Legend with counts | when mapped | *332,667 good · 304 recovered on a re-read · 3 repaired by error correction · 25 came back wrong* | **Rep** of the counts in the rows. *Repaired by error correction* is **Check and fix's** fact, drawn in Rip |
| Where-line in cm | when mapped | *Bad sectors about 4.5 cm and 4.7–5.1 cm from the centre* | **Rep** |
| Once-per-turn line | when mapped | *Read errors… repeat once per turn: single spots about 4.4 cm and 4.9 cm out* | **Rep** (a third place) |
| Disc drawing to scale | Disc view | a 400 px circle | a second round picture under the ring; mostly grey |
| Sector grid | Sectors view | ~1,000 blocks, cm scale | |

`games-rip-map-repaired` shows the worst case. Rip says *Read cleanly* and *✓ None wrong*, then directly underneath its legend says *137 repaired by error correction · 3 came back wrong*, because `sector_map` draws Check and fix's map over Rip's.

### Check and fix

| Element | Shown in | What it says | Rep / Both / Rare |
|---|---|---|---|
| Header, pill | all | Checking / Done / Needs you | |
| Header sentence, 2–3 lines | all | *It matches Redump's copy of … now: 139 sectors read again from the disc, then combined with 1 more rip…* | **Rep**: the outcome, first of three |
| Item row (label, verdict, cut off) | all | *1 SLES_047.11 · ✓ Verified: Lantern Ke…* | **Rep**, second of three; cut off at 448 px |
| Red attention row with buttons | C4 | *Fix bad sectors · Rip again and combine · Repair with the disc's error correction · Keep it anyway · Skip this disc* | offers steps it has already taken |
| *Compared with Redump*: entry, *Also under…*, table of Size / SHA-1 | all | | the proof |
| *Bad sectors read again*: 3 facts | C2–C4 | *140 failed… / 139 read correctly and put in place / 1 still bad, sector 98000* | |
| *Rips*: each rip and *Combined* | C3, C4 | *Rip 1 · 1 sector fail…; Rip 2 · 152…; Combined: matches Redump; 262 sectors differed…* | |
| *Repaired by error correction* | C4 | | **Rare** |
| *What the (first) rip reported* | all | *The drive reported no read errors, SCSI 0, C2 0*; *At least 140 read wrong: 140 with the wrong address… (MSF), 15… (ECC), 12… (EDC)*; drive name and *redumper doesn't know this drive* | **Both**: Rip's story told again. Listed after the fixes, though it happened first |
| *What it likely means*: 2–4 paragraphs | all | | **Rep**, third of three; the drive paragraph repeats the drive fact above it |
| Map heading, legend, map | when mapped | *Sectors checked, from the centre out* | sits under ~800 px of text; no ring; a different heading from Rip's |

**Rough heights at 448 px.** Rip done with the map runs to about 1,000 px, with the Disc view under the fold. Check and fix verified runs to about 1,300 px, and its map is never above the fold.

## 2. Diagnosis

1. **The same fact, said three or four times.** R4 says 771 four times. C3 gives the outcome three times: the header sentence, the cut-off row and *What it likely means*. Where the damage is appears three times, in three units.
2. **Competing focal points.** Rip has two circles: the ring, and the Disc drawing. Check and fix has three headings that each sound like the answer. The CD sidebar has one: the ring, with its headline beside it.
3. **Facts in the wrong step.**
   - Rip draws Check and fix's repairs, which happen downstream. That breaks the rule that steps know only their wires, and it makes Rip contradict itself.
   - Check and fix retells Rip's read (*What the first rip reported*). It may know this, since it came in on the wire, but Rip's sidebar already tells it.
4. **No order in time.** Rip's rows are in order. Check and fix's sections run: compare, fix, rips, repair, then the read, which came first. The order changes with what happened.
5. **Long scroll.** Paragraphs of explanation push the proof and the map below the fold. Settings already live in Settings; here the scroll is all prose.
6. **Inconsistent naming.**
   - *bad / unreadable / wrong / came back wrong / not clean / fail their own check*
   - *recovered / put right / read correctly and put in place / repaired*
   - *Check every sector* (a pass in Rip) against *Check and fix* (the next step)
   - *re-read* used in both steps for different things
   - *Every sector…* against *Sectors checked…*
   - *SCSI / C2 / MSF / ECC / EDC* in plain view
7. **Two Stops, and actions that no longer help.** The crawl warning has its own Stop beside the top bar's. C4 offers *Fix bad sectors* after it has already run.
8. **Different shapes.**
   - Check and fix has no ring, and its header carries a sentence.
   - Rip puts *Open tray* on a row of its own.
   - The CD sidebar, which the owner likes, has neither problem.

## 3. Proposal: one shape for CD Rip, game Rip and Check and fix

The same six slots, top to bottom, each only when it has something to say:

1. **Header.** Icon, step name and pill. *Open tray* goes on this row, on the right, as a quiet button (Rip and Drive only). No sentence under the header.
2. **Ring and headline.**
   - **The ring:** progress while working (*55% read*), then what was made and where it stands (*633 MB made / verified / not verified*). A CD shows *16/17 verified* in the same place. The ring never shows the same number as the headline.
   - **The headline:** the card's own status words (*Slowed by read errors*, *34 sectors couldn't be read*, *Verified*, *Doesn't match Redump*).
   - **Under the headline,** at most two short lines (where it is, time left and speed, or what happens next), plus the crawl warning when it applies.
3. **What needs you.** One box, as for a CD track that needs you. It holds one reason line, shown only when there is a reason, and only the buttons that can still help.
4. **What it's done.** One row for each thing the step did, in order. Each row has its time and a short result in sectors. A row with a note has a one-line subtitle (*20 tries each*, *rips 2, 3 and 4*). While a step is working, its current row has the spinner, so this list replaces the *Pass N* line.
5. **Where on the disc.**
   - A key with counts, using one set of words.
   - One where-line in centimetres, which also covers the once-per-turn note.
   - One picture: the sector grid, with its cm scale.
   - The heading is the same in both steps.
6. **Proof.** Redump's copy beside this one (Size, SHA-1), last. On a CD the proof is AccurateRip's column in the track rows.

### Who says what, on their wires

| Fact | Rip | Check and fix |
|---|---|---|
| Reading progress, speed, time left, crawl | ✓ | |
| Read and re-read passes, in sectors | ✓ | |
| Where the damage is, as Rip read it | ✓ (its own grid only) | |
| What it handed on | headline (*34 sectors couldn't be read · left blank for Check and fix*) | first row, muted: *← From Rip: the image, 34 sectors blank* (what came in on the wire) |
| Compare, read again, rip again, repair, compare | | ✓ rows |
| The sectors it touched, and how they ended | | ✓ its own grid |
| Redump's copy beside this one | | ✓ |
| The drive used general settings | Log | the reason line, only when it explains a hold |
| SCSI / C2 / MSF / ECC / EDC counts | Log | Log |

Rip's grid stops drawing Check and fix's map: `sector_map` takes only Rip's own record. Check and fix's grid shows only the sectors it received as bad and what happened to them.

### Words

| Word | Use for | Replaces |
|---|---|---|
| **unreadable** | the drive gave nothing back | *couldn't be read* (headline phrasing only), *skipped* |
| **read wrong** | the drive gave back data that fails the sector's own check | *came back wrong*, *fail their own check*, *bad* in the key |
| **read with errors** | C2 | *with C2 errors* |
| **put right** | read right on a re-read, or taken from another rip | *recovered on a re-read*, *read correctly and put in place* |
| **repaired by error correction** | kept: it's a different and weaker fix | |
| **Test every sector** | Rip's last pass | *Check every sector* |
| **Where on the disc** | both steps' map heading | *Every sector, from the centre out*, *Sectors checked…* |

*Bad* stays only in a row's short result (*1,344 bad*), where the key below it says which kind.

### The mockup's states

All four use illustrative counts, shaped on Koudelka (Europe) (Disc 4)'s real runs:
- 269,159 sectors, 633 MB;
- read with general settings at +103;
- two patches, about 4.4 cm and 4.9 cm out;
- ~1,300 unreadable after the first read.

The cached runs were stopped part way, so the final counts below are made up to fit and add up.

| State | Ring | Headline | Rows | Key |
|---|---|---|---|---|
| A Rip, reading through errors | 55% read | Slowed by read errors | 1 Read the disc · 926 bad so far | 147,114 good · 21 read with errors · 905 unreadable · 121,119 not read yet |
| B Rip, done, handed on | 633 MB made | 34 sectors couldn't be read | Read 1,344 bad · Re-read 1,310 put right · Write ✓ · Test ✓ | 267,815 good · 1,310 put right · 34 unreadable |
| C Check and fix, verified | 633 MB verified | Verified | ← 34 blank · Compare ✕ · Read again 31 put right · Rip again 3 from rip 2 · Compare ✓ | 269,125 as Rip made them · 34 put right |
| D Check and fix, holding | 633 MB not verified | Doesn't match Redump | ← 34 blank · Compare ✕ · Read again 31 · Rip again none · Compare ✕ | 269,125 · 31 put right · 3 unreadable |

The Redump SHA-1 is the Europe Disc 4 `.bin`'s (`3a647314…`). 269,159 × 2,352 = 633,061,968 bytes, which is its size in the dat.

The fixing and re-reading states have the same shape as A: the running row carries the spinner, and the two lines under the headline say *12 of 34 · try 3 of 10*.

What it saves: each state has its headline, actions and rows above the fold at 1280 × 720. Check and fix verified goes from about 1,300 px to about 900.

## 4. Dropped on purpose

| Dropped | Why | Where it goes |
|---|---|---|
| Check and fix's header sentence | the outcome a third time | the headline |
| *What it likely means* paragraphs | the outcome again, in prose | a row subtitle, or the one reason line |
| *What the first rip reported* | Rip's story told twice | Rip's sidebar. The input row keeps the one number that came on the wire |
| MSF / ECC / EDC breakdown; SCSI / C2 sample counts | jargon, in units nobody reads | the Log tab |
| The Disc view as the first picture | a second round picture, 420 px, mostly grey; the grid's cm scale says the same | kept as an option (see *Approved* below): the grid is the default, the drawing takes the grid's slot when chosen. Never both, and never under the ring |
| % where-line beside the ring | a second unit for the same place | the cm line under the key |
| *Pass N · …* line and disc label | the row says the pass; the Game disc card has the label | |
| Rip's separate progress bar *and* ring | the ring fills | the bar is kept only beside the headline while busy, as on a CD |
| Crawl warning's own Stop | the top bar has Stop | |
| Advice paragraph in R4 | says what the buttons say | the buttons |
| Drive name line | in the window title | |
| Check and fix's map drawn in Rip | breaks the wires rule and contradicts Rip's rows | Check and fix's own grid |
| Offers of steps already run (C4) | they can't help now | *Try another drive*, *Repair with error correction* when not yet tried |
| *Also under SERIAL: …* | rarely needed | shown only when the entries differ in more than the revision |

**Kept, because the owner likes it:** the ring with its headline beside it, the rows, the shape-coded grid with its Okabe–Ito colours, the crawl warning, the cm distances and the once-per-turn insight (as one line), and the Redump table.

## Approved

The owner approved this, with one change: the to-scale disc drawing stays, as an option. The Disc / Sectors switch is kept, small and secondary, under *Where on the disc*; **Sectors (the grid) is the default**, and the choice is still remembered (a choice already stored is kept).

As built:
- *Try another drive* is left out. Check and fix's *Rip again and combine* reads from the drive the disc was ripped on (the device is on the dump), so switching drives in the picker wouldn't change what it reads. The reason line still says another drive may help.
- *Rip again and combine* is offered again only until as many rips as Check's settings allow have been made; the other fixes that have run aren't offered again.
- Rip's ring in state B keeps the warn colour for *made with gaps*, as its card does.
- Check and fix's ring shows the copy's size when it has compared it with Redump's; a copy that matched at once shows *1/1 verified*, as a CD shows *16/17*.
- Before it has run, Check and fix keeps its one sentence on what it's for; once it has run, there's none.
