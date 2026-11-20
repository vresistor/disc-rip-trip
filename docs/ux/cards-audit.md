# Cards and drawers: audit, November 2026

Every built-in step's tile and drawer, as the window draws them today. The spec that answers this is [cards.md](cards.md).

**Renders:** 195 PNGs in `target/inventory/` (light, 1280 × 720, recipe list cropped off), named `<recipe>-<moment>-<step>[-<tab>].png`. The `index.tsv` beside them has each step's name, subtitle, status and detail as the snapshot gave them. Regenerate with:

```sh
cargo test -p drt-app --test inventory -- --ignored --test-threads 1
```

Not rendered, because the simulator never reaches them: Drive *Offset?* and *Insert a disc*; Discogs, GnuDB and TMDb *Not set up*; Video disc *MakeMKV isn't installed*. The spec rules on them from the real steps' code.

Moments: `0idle` waiting, `1ask` a question open, `2hold` held, `busy*` working, `3done` done, `bad` failed, `setup` unset or unplugged. Recipes: `cd`, `ar` (Archive everything), `da` (data), `ga` (games), `vi` (video), `keep`.

**The simulator's text.** The renders show the simulated steps. Where the real step says something else, the table gives the real text, and notes the simulator's in brackets.

**Marks:**
- **R**: redundant, because the same fact is on screen elsewhere at the same time.
- **G**: generic, because it would read the same on any step.
- **N**: noise, because it's true but useless for this step's job.
- **M**: missing, something a person watching this step would want to know.

Columns:
- **Sub** is the tile's subtitle.
- **Main** and **Detail** are its status line and detail line.
- **Pill · sentence** is the drawer header.
- **Rail** is the left-hand settings column.
- **Tabs** are the record tabs, plus the filter chips under them.

## Source and kind

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **Drive** | idle | Blu-ray drive | Ready · — | Ready · "Reads from the Blu-ray." | drive radio; offset +103 *AccurateRip* | — | **R** the drive's name four times: header, sub, sentence, rail. **R** Ready ×2. **M** the offset isn't on the tile (`cd-0idle-drive`) |
| | busy | Blu-ray drive | Reading · 8×, track 6 | Reading · "The White Stripes is in the Blu-ray." | radio row: the album name **drawn over** Busy | Log | **R** the same track progress as Rip's tile. **Bug:** overlapping text in the rail (`cd-busy2-drive`) |
| | done | Blu-ray drive | Done · — | Done · "…is in the Blu-ray." | radio row: overlap again | Log; Out 1 disc → Audio CD | **G** "Done" as the main line. **N** the Out line. **Contradiction:** after the eject the tile says *Empty* while the header says "The White Stripes, in Blu-ray drive" (`cd-2hold-canvas`) |
| | unplugged | Blu-ray drive | Not connected | Not connected · "The Blu-ray isn't connected." | row: Not connected | — | **R** five times: chip, tile, pill, sentence, row (`cd-setup-drive`) |
| **Audio CD** | idle | table of contents | After Drive | Ready · "Reads the table of contents when a disc is in." | **Uses: Table of contents** | — | **R** the sub, rail and sentence all say the same thing. The pill says *Ready* while the tile says *After Drive* (`cd-0idle-cd`) |
| | done | table of contents | 17 tracks · 43:45 | Done · "17 tracks, 43:45." | Tracks 17, Length 43:45 | Found (table), Log | **R** the track count four times. **N** the Facts table: a "Disc 17" row, then lengths in seconds on a diagonal. **M** the track list in m:ss, and no Items tab. The real step adds the disc ID to the rail; the simulator doesn't (`cd-1ask-cd`) |
| **Data disc** | done | label, filesystem, size | CD, 682 MB · ENCARTA98 | Done · "ENCARTA98" | Uses: Label, filesystem, size | Found: Bytes **681984000** | **G** the sub. **R** the label is also the sentence. **N** raw bytes, "Audio tracks 0" (`da-3done-d_disc`) |
| **Game disc** | done | system and serial | PlayStation, SLES-00860 (2 lines) · Europe, SLES_008.60 | Done · "Europe, SLES_008.60" | Uses: System and serial | Found: 9 columns, truncated, with *Redump's* chips over the values | **R** the serial twice, spelt two ways. **N** the Found table is unreadable (`ga-3done-g_disc`) |
| **Video disc** | done | MakeMKV | DVD, 3 titles · BLADE_RUNNER | Done · "BLADE_RUNNER" | Shortest title (seconds) 60 | Items: "Title 2 Left out" with no reason; Found: *Minlength*, *Makemkv id*, *Picked* | **N** engine keys. **M** why a title was left out: duplicate, trailer, too short (`vi-3done-v_disc`) |
| **Next disc** | busy (waiting for Rip) | eject, start again | — · — (spinner, blue outline) | **Waiting** · "Ejects the disc once it's read; the recipe then starts again for the next one." | — | — | **Contradiction:** the tile spins while the pill says Waiting. It looks busy but is idle (`cd-busy2-next`) |
| | done | eject, start again | Ejected · waiting for the next | Ejected · the same sentence | — | — | **G** the sentence never changes (`cd-2hold-next`) |

## Lookups

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **MusicBrainz** | idle | **lookup** | After Audio CD | Ready · "Looks the disc up on MusicBrainz." | Asked with: The disc | — | **G** sub. **G** sentence. **N** rail (`cd-0idle-mb`) |
| | busy | lookup | *(empty)* · asking | **Ready** (spinner) · the idle sentence | — | — | **Bug:** the pill says *Ready* while the step works. An empty main line (`cd-busy0-mb`) |
| | done | lookup | 2 releases · by disc ID | Done · "2 found by disc ID." | Asked with: Disc ID | Found (each row "MusicBrainz · …"), Log; In/Out | **R** the count and the method, on the tile and in the sentence. **R** the source name on every row. **M** the disc ID (only in Log), and which release Choose took (`cd-3done-mb`) |
| **CD-Text** | idle | **lookup** | After Audio CD | Ready · "Reads the titles some discs carry." | **Read from: The disc** | — | **G** sub. **N** rail (`cd-0idle-cdtext`) |
| | done | lookup | 17 titles · from the disc *(sim: artist, album)* | Done · "17 titles read from the disc." | Read from: The disc | Found: one *Title* column, every row tagged *MusicBrainz's* | **R** "from the disc" three times. **N** "MusicBrainz's" ×17. **M** the outcome: **none used, MusicBrainz's titles won**. The tile still reads as a success (`cd-3done-cdtext`) |
| **Discogs** | done | **lookup** | 1 release · by barcode *(sim: 3 releases)* | Done · "1 found by barcode." | token •••• *app-wide* | Found, Log | **G** sub. **M** the barcode it asked with. The sim's tile says 3 while its drawer says 1 (`ar-1ask-a_dg`) |
| **GnuDB** | failed | **lookup** | No answer · timed out | No answer · "**Nothing found.**" | Contact email *(blank)* · app-wide | Log | **Contradiction:** "no answer" isn't "nothing found". **Loud:** a red outline on an optional lookup that doesn't stop the disc (`ar-1ask-a_gn`) |
| **Cover art** | busy | Cover Art Archive | *(empty)* · fetching | **Ready** (spinner) · "Looks the cover up in the Cover Art Archive." | From: Cover Art Archive | — | **Bug:** the pill says Ready. **R** the archive's name three times (`cd-busy1-art`) |
| | done | Cover Art Archive | Found · 243 KB *(sim: 1200 × 1200)* | Done · "1200 × 1200, from Cover Art Archive." | Size, From, File | Found (the picture), Log; Out "1 disc, **0 MB**" | **G** "Found". **R** size and source three times. **N** "0 MB". **M** a thumbnail on the tile; the picture is two clicks away (`cd-3done-art`) |
| **Lyrics** | busy | LRCLIB | *(empty)* · asking | Ready · "Looks the disc up on **Lyrics**." | .lrc files too | — | **Bug:** the step's name stands in for the service's name (`ar-busy1-a_lyr`) |
| | done | LRCLIB | 14 of 17 · synced | Done · "**Nothing found.**" | .lrc files too | Found: *[00:01.00]…* ×14, "yes" ×14 | **Contradiction:** the tile says 14, the sentence says nothing. **N** raw LRC. **M** which 3 tracks have none (`ar-3done-a_lyr`) |
| **TMDb** | busy | **lookup** | *(empty)* · asking for "blade …" | Ready · "Looks the disc up on TMDb." | API key | — | **Bug:** the pill says Ready. The query is cut off (`vi-busy0-v_tmdb`) |
| | done | lookup | 3 candidates · 3 films, 0 seasons | Done · "**3 found 3 films, 0 seasons.**" | API key ••••-key *yours, app-wide* | Found | **Bug:** the generic sentence makes broken grammar. **N** "0 seasons". **M** what it searched for (`vi-3done-v_tmdb`) |
| **Redump** | done | by hash, else serial | Verified · by hash | Done · "1 found by hash." | Uses: By hash, else serial | Found, Log | **R** "Verified" also on Check and Choose, side by side. **R** the sub is restated in the rail (`ga-3done-g_redump`) |

## Choose

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **Choose** (audio) | idle | which release | **After all 2** | Ready · "Picks the release the disc is, from what the lookups found." | Found by: "MusicBrainz, **after audio cd**" | — | **Bug:** engine text in the rail, lower-cased. "After all 2" is engine-speak (`cd-0idle-choose`) |
| | asking | which release | Which release? · 2 candidates | Which release? · "2 releases fit this disc. The first is most likely." | Found by: MusicBrainz, 2 releases; CD-Text, 17 titles | the cards, then the tracks | **R** the question three times: tile, pill, header chip. **R** the rail repeats the lookup tiles above. **M** what tells the two cards apart: both show "✓ disc ID ✓ 17 titles" (`cd-1ask-choose-question`) |
| | done | which release | The White Stripes · 1999 | Done · "The White Stripes, 1999, US, Sympathy…, SFTRI 577." | **Uses: Which release** | Found (Chose card, "you picked"), Log; "In 1 disc from MusicBrainz, CD-Text" | **R** the release on the tile, in the sentence and on the card. **N** rail. **M** country and label on the tile; 1999 alone doesn't tell the pressings apart (`cd-3done-choose`) |
| **Choose** (game) | asking | which game | "Which game? The hash…" (cut) · 2 with its serial | **"Which game? The hash matches none."** | *(empty)* | cards | **N** a sentence inside the pill. The tile's main line wraps and is cut. An empty rail (`ga-unknown-g_choose-question`) |
| | done | which game | Colony Wars (Europe) (Disc 1) · verified | Done · "verified" | Uses: Which game | Items (1), Found | **R** the third "verified" in a row. **N** an Items tab of one (`ga-3done-g_choose`) |
| **Choose** (video) | asking | which film or show, … | Which episode is each title? | the same · "6 to sort out; the likeliest is picked for each." | — | the rows: "4 placed, 2 skipped unless you pick" | **Contradiction:** "6 to sort out" against 4 placed and 2 skipped (`vi-season-v_choose-question`) |
| | done | which film or show, … | Blade Runner · 1982, 1 title | Done · "1982, 1 title" | Uses: Which film or show, which titles | Found ("best match 0.90", then 65 %) | **G** the sub is cut. **N** the score shown two ways (`vi-3done-v_choose`) |

## Read

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **Rip** (audio) | idle | cyanrip, AccurateRip | After Cover art | Ready · "Reads each track with cyanrip, AccurateRip." | AccurateRip On/Off, First read Fast/Careful | — | **R** the sentence is the sub with a verb (`cd-0idle-rip`) |
| | waiting | cyanrip, AccurateRip | Waits for choose | **Waits for choose, in orange** | "Locked while ripping" (nothing is ripping) | — | **Orange misuse.** The lock label is wrong (`cd-1ask-rip`) |
| | busy | cyanrip, AccurateRip | 5 of 17 · Wasting My Time | **5 of 17** · "5 of 17 read" | read-only values | Items, Found, Log + chips "Needs you **0** · Verified 5 · All 17"; "offset +103, Blu-ray drive"; In/Out | **R** the count three times. **R** the offset and drive repeat the Drive tile. **N** "Needs you 0". **M** the track being read is below the fold; no time left (`cd-busy2-rip`) |
| | failed | cyanrip, AccurateRip | 16 ✓ 1 ✕ · track 9 | 1 to retry · "16 verified" | — | row: "✕ Doesn't match" + "No match in AccurateRip" | **R** the verdict twice in one row. The tool output opens at its tail, about track 17, not 9 (`cd-bad-rip`, `cd-bad-rip-log`) |
| | done | cyanrip, AccurateRip | 17 of 17 · — *(sim: verified)* | Done · "17 verified" | — | Items (17 × *Verified*); Found: "**ok: ✓ 104**" ×17 | **R** the Items list again in engine text on Found. **G** "17 of 17" doesn't say verified (`cd-3done-rip`, `-found`) |
| **Image** | busy | 20 retries, refined | Reading · ENCARTA98 | Reading · "ENCARTA98" | read-only | Items: "All 1", "25%, sector 83250 of 332999" | **R** the label is on Data disc already. **N** raw sectors; a list of one. **M** errors so far, speed, time left (`da-busy-d_image`) |
| | failed | | Bad sectors · 3 unreadable sectors | 1 to retry · "3 unreadable sectors" | | row: ✕ **Failed** · 3 unreadable sectors | **R** one state, three words: Bad sectors, 1 to retry, Failed (`da-bad-d_image`) |
| | done | | **Imaged** · — | Done · — | sliders | Items: "Done 682 MB, read cleanly" | **G** "Imaged". **M** "read cleanly" belongs on the tile (`da-3done-d_image`) |
| **Rip** (game) | busy, done | 20 retries, refined | Reading · SCES_015.64 / Imaged · ✓ 458 MB, read … (cut) | Reading / Done · the note | | Items (1) | As Image (`ga-busy-g_rip`, `ga-3done-g_rip`) |
| **Rip** (video) | busy | MakeMKV, the … (cut) | 0 of 1 · Blade Runner | **0 of 1** · "Blade Runner" | **Uses: MakeMKV, the chosen titles** | Items: "Saving to MKV file 25%" | **N** a count in the pill; the rail restates the sub. **M** the percentage and time left on the tile (`vi-busy-v_rip`) |
| | done | | 1 of 1 · — | Done · — | | Found: "Selected 1" | **N** Found (`vi-3done-v_rip-found`) |

## Check

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **Check** (data) | done | Redump | Verified · IBM PC | Done · "IBM PC" | Catalogues: All PC-readable | Items: "Done Verified: Encarta 98 Enc…" (cut); Found: a SHA-1, then the name | **M** the job's result, Redump's name for the disc, is truncated in a list of one (`da-3done-d_check`) |
| **Check** (game) | failed | Redump | Doesn't match Redump · SLUS-00152 (3 lines) | 1 to retry · "SLUS-00152" | | row: Failed · reason · *Refine* | **R** three words for one state. Refine, which re-reads the disc, is offered from Check (`ga-unknown-g_check`) |
| | done | Redump | Verified · PlayStation | Done · "PlayStation" | **Catalogues: All PC-readable** (it's a PlayStation disc) | | **R** Check, Redump and Choose all say *verified* (`ga-3done-canvas`) |

## Make: encoders and the rest

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **FLAC** (all encoders alike) | idle | level 8, verified | After Rip | Ready · — | the settings in columns | — | good: the sub is the setting (`cd-0idle-flac`) |
| | busy | | 4 of 17 · 1 encoding | Encoding · — | read-only: Compression 8, Verify On | Items + chips "Encoding 1 · Done 4 · **All 5**"; In/Out "…72 MB → Review" | **R** the rail repeats the sub. "All 5" against the tile's "of 17". A stray scroll bar looks like a divider (`cd-busy2-flac`) |
| | done | | 17 of 17 · — | Done · "17 of 17 encoded." | slider in 280 px | Items: 17 × *Encoded*; Found: 01.flac… | **R** "17 of 17" twice, and the list of 17 identical rows. **M** the total size, the ratio, the verify result; the size is only in the In/Out line (`cd-3done-flac`) |
| **MP3** | idle | V0, joint stereo | | | Mode, Quality, Stereo; ID3 "**V2.3**" | | **Bug:** `cap()` on every choice value (`tray.rs` `setting`) turns *v2.3* into *V2.3*. A stray vertical line (`cd-0idle-mp3`) |
| **ReplayGain** | busy | track and album | **Waits for all 17** (blue, spinner) · 4 so far | Waits for all 17 · "4 so far" | read-only | — | Waiting drawn as busy (`ar-busy2-a_rg`) |
| | done | track and album | **Done** · album gain *(real: album −7.3 dB)* | Done · "album gain" | Gain | — | **G** "Done" as the main line. **M** the gain itself; the sim doesn't show it (`ar-3done-a_rg`) |
| **Cue sheet** | busy | **and rip log** | *(empty)* (blue) | Ready (spinner) · — | **Track names** template; **Tracks are: Flac** | — | **Odd config:** it asks you to repeat what the encoder and folder already know. The sub is a fragment (`ar-busy2-a_cue`) |
| | done | and rip log | Written · disc.cue, rip.log | Done · "disc.cue, rip.log" | the template, cramped | Items: Cue sheet, Rip log; Found: "0 MB" ×2 | **R** the file names three times. **N** "0 MB" (`ar-3done-a_cue`) |
| **CHD** | busy | default codecs, … (cut) | Compressing · — | Compressing | read-only | Items: "Compressing 25%" | **M** the percentage on the tile, beyond the bar (`da-busy-d_chd`) |
| | done | | Made · 412 MB, 60% of the … (cut) | Done · "412 MB, 60% of the image, verified: every track matches the dump (1)" | | Items (1), Found: the file and its size | **G** "Made". **R** the note three times: tile, sentence, row. An engine sentence in the header (`da-3done-d_chd`) |
| **Keep as is** | done | as dumped, with its … (cut) | As dumped · 3 files | Done · "3 files" | Uses: As dumped, with its log | Items (1) | **R** the main line repeats the sub (`keep-3done-d_chd`) |
| **LibCrypt** | done | Redump's SBI, for … (cut) | Not protected · not protected | Done · "not protected" | Uses: Redump's SBI… | Items (1) | **R** the same words three times (`ga-3done-g_sbi`) |
| **M3U** | done | playlist for multi-dis… (cut) | Disc 1 of Colony Wars (Europe) · waits for another disc | **Done** · "waits for another disc" | Uses: Playlist… | Items (1) | **Contradiction:** Done, yet waiting (`ga-3done-g_m3u`) |
| **Transcode** | busy | H.265 CRF 22, slow | *(empty)* (spinner) | **Ready** · — | read-only | Items: "Encoding" | **Bug:** the pill says Ready. **M** the percentage and time left on the job most worth watching (`vi-busy-v_make`) |
| | done | | 1 of 1 · — | Done · — | | Items: Done | **M** the size, and the saving over the rip (`vi-3done-v_make`) |

## Place, hold, tell

| Step | State | Sub | Main · Detail | Pill · sentence | Rail | Tabs | Marks |
|---|---|---|---|---|---|---|---|
| **Folder** | idle | ~/Music/FLAC | After Hold | Ready · "Files go into ~/Music/FLAC." | Folder *yours, 19 Nov* Reset; Names: preset, template, example ("**The Cinematic Orchestra/Every Day**…"), tokens | — | **R** the path three times. **N** "yours" on a built-in value. **N** an example from another disc (`cd-0idle-lib1`) |
| | unset | *(empty)* | Folder? | Folder? · "Where do the files go?" | "not set" | — | **R** the question four times (`cd-setup-lib2`) |
| | waiting | | Waits for hold | **orange** Waits for hold | | | **Orange misuse** (`cd-2hold-lib1`) |
| | busy | ~/Music/Review | 4 of 17 | **4 of 17** · "**4 of 4** filed in ~/Music/Review." | | chips "To file **0** · Filed 4 · All 4"; "**Unknown artist** / The White Stripes /" | **Contradiction:** 4 of 17 against 4 of 4. **N** a count in the pill (`cd-busy2-review`) |
| | done | ~/Music/FLAC | 17 of 17 | Done · "17 of 17 filed in ~/Music/FLAC." | template, cramped, tokens cut | **Items** says "Unknown artist/The White Stripes/…" in the order they arrived (1, 5, 6, 2…); **Found** says "~/Music/FLAC/The White Stripes/The White Stripes" | **R** two lists of the same 17 files. **Contradiction:** they disagree about the folder. **R** the path five times: sub, sentence, rail, Found heading, Open folder (`cd-3done-lib1`, `-found`) |
| | done (a cue and log) | ~/Archive/CD | 2 filed | | | Items: "1 Cue sheet **2:30 01 - Jimmy the Exploder**" | **Bug:** non-track files are drawn as tracks (`ar-3done-a_arc`) |
| | done (a disc image) | ~/Images | 1 filed | "1 of 1 filed in ~/Images." | the games recipe's example says "**IBM PC/Encarta 98**" | Images 1 | **N** an example from another kind of disc (`ga-0idle-g_lib`) |
| **Hold** | idle | until you let it through | **Holds for you** | **Holds for you** · "Keeps what arrives until you let it through." | Waiting in ~/Music/Review | — | **R** the sub and sentence say the same. The pill claims a hold before anything has arrived (`cd-0idle-hold`) |
| | filling | | **Holds for you, in orange** · 4 waiting | Holds 4 for you · "4 in Review so far." | | | **Orange misuse:** nothing to do yet (`cd-busy2-hold`) |
| | needs you | | Waits for you · 17 waiting | Holds 17 for you · "All 17 are in Review. Listen there, then let them through." + **Let 17 through** | Waiting in ~/Music/Review | Items + chips "Waiting 17 · Let through 0 · All 17"; "~/Music/Review / **Unknown artist** / …"; In 17 files from Review | **R** 17 said six times; Review said four times (`cd-2hold-hold`) |
| **Tell me** | waiting | when all done | Waits for hold | **orange** Waits for hold | When, On this computer, ntfy | — | **Orange misuse** (`ar-busy2-a_tell`) |
| | done | when all done | Told you · 17 tracks filed, all … (cut) | Done · "17 tracks filed, all verified" | When **Always** | — | The sub "when all done" fights the setting "Always". **M** where it told: desktop, ntfy (`ar-3done-a_tell`) |

## Cross-cutting problems

| # | Problem | Where | Screens |
|---|---|---|---|
| 1 | **One fact, four to six times.** The tile, pill, sentence, rail, chips and In/Out line each restate the count, the path or the state. | Audio CD's tracks ×4; Hold's 17 ×6; Not connected ×5; a folder's path ×5; Choose's question ×3 | `cd-1ask-cd`, `cd-2hold-hold`, `cd-setup-drive`, `cd-3done-lib1` |
| 2 | **The pill isn't one vocabulary.** It says *Ready* on busy steps, gives counts ("5 of 17", "0 of 1"), sentences ("Which game? The hash matches none.") and its own words (*Ejected*, *Compressing*). It disagrees with the tile (Next disc spins, the pill says Waiting). | MusicBrainz, Cover art, Lyrics, TMDb, Transcode, Cue busy; Rip, Folder, video Rip | `cd-busy0-mb`, `vi-busy-v_make`, `cd-busy2-rip` |
| 3 | **Orange where nothing needs you.** Waiting pills are amber: `Tone::of` maps Needs, Hold and Wait to `Warn` (`widgets.rs`). Hold shows amber *Holds for you* before anything arrives and while files are still coming in. | Rip, Cover art, Library, Tell me waiting; Hold idle and filling | `cd-1ask-rip`, `cd-busy2-hold`, `ar-busy2-a_tell` |
| 4 | **Blue for things that only wait.** Next disc, Cue sheet and ReplayGain spin with a blue outline while they wait for Rip to end. Mid-rip, six of the CD recipe's tiles are blue; the one reading isn't the loudest. | | `cd-busy2-canvas100`, `ar-busy2-a_rg` |
| 5 | **Busy tiles with no main line.** Lookups, Cover art, Cue sheet and Transcode leave the bold line empty, with "asking" or "fetching" small below. The working tile is the emptiest. | | `cd-busy0-mb`, `vi-busy-v_make` |
| 6 | **Subtitles by category.** *lookup* ×6. Elsewhere the sub is fine (FLAC, Folder, Image) or a fragment (*and rip log*), and at 152 px half of them are cut: *default codecs, …*, *MakeMKV, the …*, *which film or show, …*, *Redump's SBI, for …*. | | `ar-0idle-canvas`, `vi-3done-canvas` |
| 7 | **"Uses:" rail.** A step with no settings restates its subtitle as a fact (*Uses: Which release*, *Read from: The disc*, *Asked with: The disc*). It is the thinnest thing in the drawer and sits in its most prominent column. | Audio CD, CD-Text, the lookups, Choose, Keep, LibCrypt, M3U, video Rip | `cd-3done-choose`, `cd-1ask-cdtext` |
| 8 | **Two rows of tabs.** Items / Found / Log, then filter chips under them (Needs you / Verified / All). Zero-count chips are shown (*Needs you 0*, *To file 0*, *Encoding 0*), and steps with one item get *All 1*. | Rip, encoders, folders, Hold; Image, Check, CHD, Keep, LibCrypt, M3U, game Choose | `cd-busy2-rip`, `da-3done-d_image` |
| 9 | **The In/Out line earns nothing.** It's noise in every render: "Out 1 disc → Choose", "0 MB", "In 1 disc from MusicBrainz, CD-Text, Discogs, GnuDB" (one disc, not four). Its one useful number, the bytes written, belongs to the step that wrote them. | every drawer with a record | `cd-1ask-mb`, `ar-3done-a_ch` |
| 10 | **Engine text leaks.** "ok: ✓ 104", *Bytes 681984000*, *Length (s)*, *Minlength*, *Makemkv id*, *Picked*, "after audio cd", "[00:01.00]…", "sector 83250 of 332999", "best match 0.90" | Found tabs, the Choose rail, the Image row | `cd-3done-rip-found`, `vi-3done-v_disc-found` |
| 11 | **Contradictions.** | Lyrics 14 against "Nothing found"; GnuDB "No answer" against "Nothing found"; a Folder's Items against its Found; Review 4 of 17 against 4 of 4; M3U Done but waiting; the Drive *Empty* while the header says the disc is in; video Choose 6 against 4 + 2 | above |
| 12 | **Hierarchy.** The 14–15 px bold status outweighs the 13.5 px name, which is right while working but wrong when done: a finished canvas is a wall of "17 of 17" (`cd-3done-canvas`). Idle tiles are 60 % blank between subtitle and "After …". Check, Redump and Choose read "Verified / Verified / verified" in a row (`ga-3done-canvas`). | | |
| 13 | **The 280 px rail.** It cramps what needs width: template editors (tokens cut, the example wraps to three lines), the 0–8 slider, the Drive rows (the album name drawn over *Free*/*Busy*). A scroll bar 12 px from the edge looks like a divider. A step that's running shows a grey "Locked while ripping" over plain text, even before ripping. | Folder, Cue, encoders, Drive | `cd-3done-lib1`, `cd-busy2-drive`, `cd-1ask-rip` |
| 14 | **Drawers taller than their content.** An idle step with one fact opens to 220 px, mostly empty. | Audio CD, CD-Text, lookups, Choose idle | `cd-0idle-cd` |
| 15 | **Provenance where nobody changed anything.** A seeded built-in folder reads *yours, 19 Nov · Reset*, and the data Library shows it twice. | every built-in Folder | `da-0idle-d_lib` |
| 16 | **What draws the eye first.** On a busy canvas: a red GnuDB, an amber Hold and amber waiting pills, ahead of the step that's reading. In a drawer: the pill, then a sentence that repeats it, while the answer sits in a list of one below. | | `ar-1ask-canvas`, `da-3done-d_check` |

## Seen only in the simulator

These are the simulator's own faults, not the app's. They're listed so nobody chases them.
- **Discogs:** the tile says 3 releases but the step offers 1.
- **CD-Text:** the detail "artist, album" appears with no artist or album found.
- **Video recipe:** the Drive log says "audio, 17 tracks".
- **Cue sheet:** names "disc.log", but the tile says "rip.log".
- **Cover art:** the image can't be shown.
- **CHD:** a hard-coded note says 412 MB while the file is 432 MB. The real step uses one size for both.

The *Unknown artist* paths are the window's preview template running on the simulator's facts. The real disc's album artist would fill them, but the preview and the real path still come from two code paths, which is how they came to disagree.
