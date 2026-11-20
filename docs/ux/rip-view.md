# Rip's view

Rip's drawer, during and after a run. It follows the quorum (`config-quorum-space.md`): the drawer holds the run only, at full width, with no rail, and opens at half the canvas. It uses the cards spec's type (13 · 16 · 20), spacing (4 · 8 · 12 · 16 · 24) and the strip's legend. Mockup: [rip-view.html](rip-view.html), at the real drawer size for 1280 × 720 (1048 × 335) and 1920 × 1080 (1688 × 515), in four states, light and dark, plus the Drive drawer redone the same way.

**One full-width composition, with no side column.**
- A band across the top holds the disc ring, the live readout and the read-speed trace.
- The track rows run below it, at full width.
- A track that needs you opens in its own row.
- Nothing sits in a left-hand bar. Per the quorum's rule, a split is only for two halves read together in one task, and none are here.

## The idea

The archive is only as good as its proof. The view puts that proof on screen while the disc spins, so you don't have to dig in a log afterwards.

| Part | Where | What it shows | Why |
|---|---|---|---|
| **The disc** | the band's left end, a ring 88 px at 720 and 128 at 1080 | one arc per track, sized by its length, in the strip's legend: green verified, blue reading (filling as it goes), orange to retry, dark grey kept, light waiting. The centre has the outcome, *6/17* or *17/17*, with *verified* under it | the disc as you hold it. A 9-track or a 30-track disc both read at a glance: the arcs only get thinner. A short hidden track, or a 20-minute closer, shows as its true size |
| **Now** | the band, beside the ring | reading: the track and title (16 / 600), its bar, *1:17 of 2:13 · 56 s left*, and the read-speed trace. Needs you: the two reads compared. Done: the summary | the one thing happening, said once |
| **Speed trace** | the band's right end, a third of the width | the read speed over this track; when done, over the whole disc, with a short tick at the foot between tracks. A scratch shows as a dip, and the disc trace shows *where* | a disc you can't see the surface of; the dip is the scratch |
| **Rows, with proof** | below, 24 px, under one header | track, title, length, AccurateRip (*✓ Verified · 104*), CRC (tabular, tertiary), and reads (*×3* only when more than one) | the archive evidence, per track. Column headers are used here because the numbers would otherwise be ambiguous |
| **A track that needs you** | its row comes first and opens in place | each read's mode and CRC, and a bar the length of the track with the stretch that differs marked in red (*312 samples differ, 1:02–1:04*). The retries are in the header: **Read again**, *Combine reads*, *Keep*, *Leave out* | you decide with the evidence in front of you: one short scratch at 1:02 says read again, carefully |

Kept calm:
- One accent at a time. Blue moves only while reading, and *Reduce animations* stills the fill and the trace.
- No new colours: the arcs use the strip's legend.
- Hovering an arc highlights its row, and the reverse.

Judged and left out:
- **A waveform:** pretty, but it isn't proof, and it costs decoding.
- **A per-sector C2 error map:** cyanrip doesn't report one.
- **Confetti when all verified:** not calm. *17/17 verified* in the ring's centre is the celebration.

## By state

| | Ring centre | Now | Header |
|---|---|---|---|
| **Reading** | *6/17* · read | track 6, bar, time left, speed trace | pill *Reading*, sentence *about 23 min left* |
| **One to retry** | *16/17* · verified | *All 17 read*; the disc's speed trace with the dip. Track 9's row opens with its two reads compared | pill *Needs you*, sentence *Track 9 doesn't match AccurateRip.*, the retries |
| **Done, all verified** | *17/17* · verified | *Read in 24:51 at 8.1×*, the disc's speed trace, *confidence 103–104 of 104 · offset +103* | pill *Done*, no sentence |
| **Done, one kept** | *16/17* · verified · *1 kept* | as done, plus *Track 9 kept unverified: two reads disagree at 1:02–1:04* | pill *Done*, sentence on the kept track |

## What Rip reports, and what it would need to

Rip may only show its own inputs and outputs (the wires rule).

| Shown | Source today | New? |
|---|---|---|
| Track lengths, offset | the disc item (Audio CD's lengths; Drive's offset) | no |
| Each track's state and verdict, confidence | `ItemView.note` (*✓ 104*), `verified` | no |
| Progress in a track, time left | the item note (*reading 40%*), the detail (*· 56 s left*) | no |
| Each read's mode and CRC | a fact per read, as a string (*read 2, paranoia 3: Mismatch, CRC 9B03E6A1*) | **structured:** `{read, paranoia, verdict, crc}` per read, so the view doesn't parse prose |
| Where two reads differ | `Reads::damage()`, as a note string (*312 samples differ, 1:02–1:04*) | **structured:** `{samples, from, to}`, for the bar |
| When each track was read | log lines, timestamped (*track 6 read 1 …*) | no; derived from the log |
| Read speed over time | not reported; cyanrip prints only an average speed per track | **new:** sample speed from the progress callback's timing (progress × track length ÷ elapsed), e.g. twice a second, kept with the run |
| AccurateRip's best confidence for the disc (*of 104*) | cyanrip prints *disc found in database (max confidence: 104)*; not parsed | **new:** parse it in `cyanrip.rs` |
| AccurateRip v1/v2 CRCs | cyanrip prints *Accurip v1: … / v2: …*; not parsed | **new, optional:** for the Found tab, not the rows |

Everything new is a fact on the track item, or one series on the step's record (the speed). It needs no new window code beyond the view itself.

## The Drive drawer, the same way

- **Run only, full width:**
  - the band shows whether a disc is in, what's reading it, and the speed trace;
  - below it, the drive, its model and the offset.
- **Which drive** is a dropdown only when more than one drive is connected. With one, it's plain text: *the only drive connected*.
- **The offset is read-only**, with its source (*from AccurateRip, 528 agree*). Choosing a drive and *Measure* belong to its settings, not the run.
- **No *Eject* while a step is reading.** It appears only when the drive is idle with a disc in.
- **What's new to report:** nothing. The drive's state, what's reading it, and its speed come from Rip's progress, and the offset and its source are already settings with provenance.
