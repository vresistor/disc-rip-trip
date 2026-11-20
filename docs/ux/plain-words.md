# Plain words

An audit of every word the window shows, against one test: would someone
ripping their own CD know what it means, or what clicking it does?
November 2026.

## How to apply this

- **Only displayed text changes.** Kinds, ids and param keys stay as they
  are (`combine`, `leave_out`, `tell`, `first_read`), so saved recipes and
  settings still load.
- **⚑ marks text the code also reads.** Some of these are choice values
  saved in recipes and compared in code (`WHEN[1]`, `chd::CODECS[0]`,
  `"fast"`). They need a display map, or an alias like
  `template.rs`'s `"As on the NAS"`. Others are notes and statuses matched
  by `contains`, `starts_with` or `==` elsewhere, such as
  `tray.rs::rip_verdict`, `card.rs`, `short_settle` and `cue.rs` reading
  `"ok: …"`. Change both sides.
- **The simulators repeat these strings.** `sim.rs`, `game_sim.rs` and
  `video/sim.rs` need the same renames.
- **`.from(...)` is one short phrase.** It shows in the + picker, on a
  twin tile as "Name, from", and inside one sentence: `tray.rs` says
  "Looks the cover up in the {from}." So Cover art's `.from` has to stay
  a name that fits that sentence. The picker cuts it off at about 40
  characters (`popover.rs`, `WIDTH - 56`), and every proposal here fits.
  It is also searched, so "lossless" finds FLAC.
- **Some old rulings are overridden.** `GLOSSARY.md` and
  `heuristic-review-2026-11.md` chose *Consistent*, *Combined*,
  *confidence*, *Left out*, *Image* and *title* (for a DVD title). Where this doc replaces them, update the
  glossary, README.md and `docs/audio-accuracy.md` to match.
- **App code is cited by function or string, not line number,** because
  `app/` is being rebuilt.

## 1. Step names and picker descriptions

| Where | Now | Proposed | Why |
|---|---|---|---|
| `data.rs::image_info` name | Image | **Rip** | It's the same job as CD and game Rip, and Games already calls this exact code *Rip*. One word for reading the disc into files. |
| `data.rs::image_info` from | redumper | copy the whole disc to an image | A tool name says nothing about what happens. |
| `game.rs::rip_info` from | (inherits redumper) | copy the whole disc to an image | Same. |
| `data.rs::keep_info` name | Keep as is | **Plain image** | *Keep* is also a retry and a dialog button. This step is the uncompressed choice beside CHD. |
| `data.rs::keep_info` from | as dumped, with its log | uncompressed image, with its log | "Dumped" is insider talk. |
| `game.rs` / `game_sim.rs` LibCrypt name | LibCrypt | **Copy protection** | Only emulator people know the name. Keep *LibCrypt* in the description. |
| LibCrypt from | Redump's SBI, for PAL PlayStation | LibCrypt file, for European PlayStation | SBI and PAL are jargon. |
| `transcode.rs` name | Transcode | **Smaller video** | "Transcode" is jargon. Its point is a smaller file. |
| `transcode.rs` Remux name | Remux | **Original video** | "Remux" is jargon. It pairs with *Smaller video*. |
| Remux from | video as it is | the video exactly as on the disc | Clearer. |
| Transcode from | (summary) | smaller files, H.265 or AV1 | Says what it's for. |
| `real/mod.rs` Rip (CD) from | cyanrip, AccurateRip | read each track and check it | Tool names. |
| `video/mod.rs` Rip from | MakeMKV, the chosen titles | copy the chosen films and episodes | Tool name. *Title* is changed below. |
| `video/mod.rs` Video disc from | MakeMKV | list the films and episodes on it | Tool name. |
| Audio CD from | table of contents | read the track list | "Table of contents" is CD-spec talk. |
| Data disc from | label, filesystem, size | read the disc's name and size | "Filesystem" is jargon. |
| Game disc from | system and serial | which system, and its serial number | Everyday words. |
| MusicBrainz from | lookup | free online music database | "lookup" is the same for four steps and tells them apart by nothing. |
| Discogs from | lookup | record collectors' database | Same. |
| GnuDB from | lookup | older CD database | Same. |
| CD-Text from | lookup | track names stored on the disc | Same. |
| TMDb from | lookup | find the film or show online | Same. |
| Lyrics from | LRCLIB | words for each track, from LRCLIB | Says what it fetches. |
| Lyrics file from | .lrc beside each track | a lyrics file beside each track | Puts the extension second. |
| Cue sheet from | and rip log, beside the files | track list and rip log, beside the files | "and …" doesn't read on its own in the picker. |
| ReplayGain from | album | even out volume between tracks | "album" alone means nothing. |
| CHD from | (summary "default codecs") | compressed image for emulators | Says what it's for. |
| Redump from | by hash, else serial | find the game in Redump's list | "Hash" is jargon. |
| Check from | Redump | compare with Redump's known-good copies | Says what checking means. |
| Choose from | which release | pick which album this is | "Release" is a MusicBrainz word. |
| Game Choose from | which game | pick which game this is | Matches the CD wording. |
| Video Choose from | which film or show, which titles | pick the film or show, and its videos | *Title* is changed below. |
| Hold from | until you let it through | keeps files until you say they're fine | "It" has nothing to refer to in a picker. |
| Next disc from | eject, start again | open the tray, start on the next disc | Plainer. |
| Encoders' from | FLAC, MP3, Opus, AAC, Vorbis and WavPack show their settings summary (e.g. "V0, joint stereo") | FLAC: no quality lost, smaller than WAV. MP3: plays on anything. Opus: small files, good sound. AAC: for Apple and most phones. Vorbis: open format, like MP3. WavPack: no quality lost. ALAC: no quality lost, for Apple. WAV: no compression, biggest files. | The picker is where someone chooses a format. Settings mean nothing yet there. |
| `popover.rs` group | Source | Drive | It holds Drive and Next disc. |
| `popover.rs` group | Read | Rip | One word for the step. |
| `popover.rs` group | Place | Save to | "Place" is internal. |
| `popover.rs` group | Tell | Notify | Left over from the Notify rename. |
| `tray.rs` a11y | Tile name | Step name | *Tile* is a design word. The README also says *tile*: change it to *step* there. |

## 2. Buttons and actions

| Where | Now | Proposed | Why |
|---|---|---|---|
| Rip retry `combine` | Combine reads | **Best of 3 reads** | The owner's example. It reads up to 3 times and keeps what most reads agree on. |
| Rip retry `slowly` | Slowly | Read again slowly | A bare adverb isn't an action. |
| Sim retry `other_drive` | On the other drive | Try the other drive | Same. |
| Rip / Image / Check retry `keep` ⚑ | Keep as is (the button shows "Keep") | **Keep it anyway** | It keeps an unverified read. *Keep* alone also means "don't" in the dialogs. `short_settle`'s `" as is"` strip has to go. |
| Rip retry `leave_out` | Leave out | **Skip this track** | The owner's example. |
| Video Rip retry `leave_out` | Leave out | Skip this video | Same, for a video. |
| Image / Check retry `leave_out` | Leave out | Skip this disc | Same, for a disc. |
| Image / Check / game retry `refine` | Refine (hint "re-reads bad sectors") | Re-read bad parts | "Refine" is redumper's word. |
| `tray.rs` RetryAll | Read all again | Read them all again | "All" is ambiguous: it could mean the whole disc. |
| Quit dialog | Keep | Keep ripping | *Keep* means a track elsewhere. Here it means "don't quit". |
| Delete dialog | Keep | Don't delete | Same. |
| Remove-step confirm (`Act::Keep`) | Keep | Don't remove | Same. |
| Canvas | Fit | Fit to window | A bare verb doesn't say what it fits. |
| Drive drawer and setting | Measure / Measuring | Measure offset / Measuring… | Measure what? |
| Header link | Working | Working files | A bare adjective. It opens the run's folder. |
| Rip drawer | Working folder | Working files | One name for that folder. |
| Header | Latest disc | Back to current disc | Says where it goes. |
| Recipe actions | Reset to built-in | Reset to original | "Built-in" is developer talk. README says it too. |
| Hold drawer | Let through / Let N through | Let them through / Let N through | Fine with a count. Bare, it needs an object. |

## 3. Settings and values

| Where | Now | Proposed | Why |
|---|---|---|---|
| Drive `offset` on the tile | Offset | Read offset | The drawer already says *Read offset*. One label per setting. Add a tooltip: "How far this drive's audio is shifted. Needed to check a rip." |
| Rip `tracks` | Tracks | Tracks to rip | Says what the value does. |
| Cover art `size` on the tile | Largest | Cover size | "Largest" isn't a noun. |
| Check `systems` | Catalogues | Look in | Plain. |
| Check value ⚑ | All PC-readable | All systems | "PC-readable" is an internal idea: the systems a PC drive can read. |
| CHD `codecs` on the tile | Codecs | Compression | The drawer says *Compression*. |
| CHD value ⚑ | Default for the disc | Standard | Plain. It pairs with the summary. |
| CHD summary | default codecs | standard compression | No *codec*. |
| CHD `hunk` (Advanced) | Hunk size | Block size | "Hunk" is MAME's word. |
| FLAC `level` on the tile | Level | Compression | The drawer says *Compression*. Tooltip: "Higher is smaller and slower. The sound is the same." |
| FLAC summary | level 8 | compression 8 | Same. |
| MP3 / Opus `mode` | Mode | Bitrate type | "Mode" of what? |
| MP3 / Opus values ⚑ | VBR / CBR / ABR / Constrained | Variable / Constant / Average / Mostly constant | The acronyms, spelled out. |
| MP3 `quality` | V0…V9 | (keep) + tooltip "V0 is best" | V0 is the name MP3 users know. It's unreadable without a hint. |
| MP3 `stereo` | Joint / Full | (keep) + tooltip "Joint is smaller and sounds the same" | Same. |
| MP3 `id3` | ID3 | Tag version | Says what it is. Values v2.3 and v2.4 stay. |
| MP3 `id3v1` | ID3v1 as well | Old-style tags too | Same. |
| Opus `complexity` (Advanced) | Complexity | Effort | Plain. |
| Notify `ntfy_topic` | ntfy topic | Phone alerts (ntfy topic) | Says what ntfy is for. |
| Video disc `min_length` | Shortest title (seconds) | Skip videos shorter than (seconds) | Says what it does. *Title* is changed below. |
| Video disc on the tile | Shortest | Skips under | Same. |
| Transcode `crf_*` | Quality (CRF) | Quality (lower is better) | CRF goes in the tooltip. |
| Transcode `preset_h265` | Preset | Speed | x265's own values stay (ultrafast…veryslow). |
| Transcode `preset_av1` | Preset (lower is slower) | Speed (lower is slower, smaller) | Same. |
| Transcode `codec` value ⚑ | Copy | As on disc | *Copy* reads as "make a copy". |
| Transcode `audio` value ⚑ | Copy | As on disc | Same. |
| Name presets ⚑ | Date and label | Date and disc name | *Label* is a record label in music templates. |
| `DUMP_TOKENS` description | label: the volume label / name: …or else the volume label | the name stored on the disc | "Volume label" is jargon. The token key `label` stays. |
| Name presets | Sorted artist / Album / 01 Artist - Title | (keep) + tooltip "The Beatles files under Beatles, The" | The tooltip explains what *sorted* means. |

## 4. Statuses and messages

### Rip verdicts (CD)

| Where | Now | Proposed | Why |
|---|---|---|---|
| `rip_verdict` detail | Verified · confidence 12 | Verified · matches 12 other rips | That's what AccurateRip confidence counts. |
| `rip_verdict` pill ⚑ ("two reads agree") | Consistent · two reads agree | **Reads agree** · AccurateRip couldn't confirm it | "Consistent" says nothing about why. Overrides the glossary. |
| Rip note ⚑ | ✓ combined from N reads | ✓ best of N reads | One name for the retry and its result. |
| Rip note ⚑ (`starts_with("combined")`) | combined from N reads, M samples unsettled | best of N reads, M samples uncertain | "Unsettled" is invented. |
| `rip_verdict` pill ⚑ | Combined | Best of N | Same. |
| `rip_verdict` pill and note ⚑ | Kept · unverified / "kept, unverified" | Kept anyway · not verified | Matches the button. |
| `rip_verdict` detail | Reads disagree: N samples differ, 1:02–1:05 | Reads differ at 1:02–1:05 (N samples) | When first, then how much. |
| `rip_verdict` detail | Doesn't match · No match in AccurateRip | Doesn't match · other people's rips of this disc differ | Says what it doesn't match. |
| `rip_outcome` qualifier | 2 by agreeing reads | 2 by matching reads | Shorter and plainer. |
| Rip busy note | reading for a vote | reading again (best of 3) | "Vote" is how it works inside, not what the user sees happen. |
| Drive status | Offset unknown | Read offset unknown | One name. |
| Offset setting | Not measured: it needs a disc in the drive that AccurateRip knows | Not measured yet: put in a well-known CD, then Measure offset | Says what to do. |
| Offset provenance | measured, confidence N | measured, N matches | No *confidence*. |
| Offset provenance | the old app | an earlier version of the app | "The old app" is the owner's own history. |
| MusicBrainz qualifier | by layout | by track lengths | Says how it matched. |
| GnuDB status | Asking by freedb ID | Asking by disc ID | Only insiders know freedb. MusicBrainz already says *disc ID*. |

### Lookups and Choose

| Where | Now | Proposed | Why |
|---|---|---|---|
| Lookups' empty status | None (Cover art, Lyrics, Discogs, GnuDB, CD-Text) / Not found | Nothing found. CD-Text: None on the disc | Three words for one result. |
| Choose prompt | Which release? | Which album is this? | The say line already lists what tells them apart (year, country). |
| Choose / Video choose count | N fit / N candidates | N found | One word. *Candidate* is internal. |
| `tray.rs` choose say | One fits; check it's the one. / One match; … / N fit; the first is most likely. | One found; check it's right. / N found; the first is most likely. | Same. |
| Choose skipped | Skipped · No release | Skipped · not named | "Release" again. |
| MusicBrainz / Discogs / GnuDB outcome | N release(s) | N found | One word for what a lookup found. |
| Cover art status | None · no release chosen | Nothing found · no album chosen | *Album* is the word for what Choose settles. |
| Choose track check (`tray.rs`) | Tracks, {year} release, against the disc / not in this release | Tracks, {year} edition, against the disc / not on this edition | *Edition* is the word for one pressing of an album. |
| CD-Text say | Offers the titles some discs carry. / Offers the disc's own N titles. | Reads the track names some CDs carry. / Found N track names on the disc. | *Title* means something else on a DVD. |
| Game Choose prompt | Which game? The hash matches none. | Which game? None matched exactly. | No *hash*. |
| Game Choose count | N with its serial | N with this serial number | Plainer. |
| Redump lookup | 1 entry / No entry / matches by hash | 1 match / Not listed / exact match | Plainer. |
| Redump lookup | No catalogue · unknown system | Nowhere to look · unknown system | Plainer. |
| Check verdict | N catalogues | looked in N systems | Matches the *Look in* setting. |

### Data discs and games

| Where | Now | Proposed | Why |
|---|---|---|---|
| Image note ⚑ ("refined") | ✓ 682 MB, refined | ✓ 682 MB, bad parts re-read | Matches the button. |
| Image note | imaged, but N sectors read wrong (EDC 3, ECC 1) | read, but N sectors came back wrong | The error codes go to the log only. |
| Keep family ⚑ (`contains("unverified")`) | keeping as is / kept as is, {size} / Kept as is · doesn't match Redump / {k} kept / track N kept unverified | keeping anyway / kept anyway, {size} / Kept anyway · doesn't match Redump / {k} kept anyway / track N kept, not verified | Matches the *Keep it anyway* button. |
| Image note | kept, N unreadable sectors filled | kept anyway, N unreadable sectors left blank | Says what "filled" means. |
| redumper phases | Refining / Splitting / Hashing / Checking protection | Re-reading bad parts / Writing tracks / Checking / Checking copy protection | Plainer. |
| Check status | Hashing N files | Checking N files | No *hash*. |
| Game status | Refining | Re-reading bad parts | Same. |
| CHD status | Compressing, zstd, lzma | Compressing | Codec names go to the log. |
| LibCrypt status ⚑ ("beside it") | SBI written | Protection file saved | No *SBI*. |
| Game refusal | …encrypted per title, and nothing here decrypts it yet, so the dump wouldn't play. | …locked to each game, and this app can't unlock it yet, so the copy wouldn't play. | Plainer. |

### Notify, Hold and the window

| Where | Now | Proposed | Why |
|---|---|---|---|
| Notify status | Told you / Didn't tell you / Couldn't tell you | Sent / Not sent / Couldn't send | Left over from *Tell me*. |
| Hold status and pill ⚑ | Holds for you | Holding | It sits beside *Waits for you* and *Needs you* for different things. |
| Hold detail ⚑ (`WAITS_FOR_YOU`) | Waits for you | All here | It means every file has arrived. |
| Skipped items ⚑ | Left out (pills, `Cell::Out` "left out", `ItemState::Skipped` notes) | Skipped | Two words for one state. *Skip* is now the button. |
| `snapshot.rs` recipe status | {n} to retry | {n} need you | Rip already says *N tracks to decide*, and the pill says *Needs you*. "Retry" is a developer word. |
| `check.rs` errors (shown when an edit is refused) | a_rip takes Track but is given Dump / … needs … found before it / … goes straight after … | {Step} can't go here: it needs {plain kind} before it | They name step ids and Rust types. Use step names and plain kinds (tracks, an image). |
| `run.rs` start refusal | not set up: Offset on drive | Missing: Read offset, on Drive | Uses the step's name and one label. |
| MakeMKV missing | MakeMKV isn't installed · flatpak install flathub com.makemkv.MakeMKV, or makemkv.com | MakeMKV isn't installed · get it from your software store or makemkv.com | A command line isn't an everyday instruction. |
| Unavailable step | This step isn't in this build. | This step isn't in this version of the app. | "Build" is developer talk. |
| ReplayGain item | WAV holds no tags; as it was | WAV can't store volume info; left as is | Plainer. |
| Encoders' errors, CHD and MakeMKV | ffmpeg: …, the CHD doesn't decompress…, redumper's last line | Prefix "Couldn't make the file:" and keep the tool's text after it | Raw tool output can't be avoided, but the first words should be plain. |

### Video (title becomes video)

| Where | Now | Proposed | Why |
|---|---|---|---|
| `video/*` everywhere | Title N, Listing titles, Couldn't read the titles, N of M titles, Which title is the film?, Which episode is each title?, Nothing to rip · no titles chosen | Video N, Listing videos, Couldn't read the videos, N of M videos, Which video is the film?, Which episode is each video?, Nothing to rip · no videos chosen | *Title* already means a song's or a film's name. This is a DVD title, which is a video on the disc. |
| Video choose | Extra · kept with the bonus material | (keep) | Already plain. |

### README.md

| Where | Now | Proposed | Why |
|---|---|---|---|
| What it does | *Read again*, *Combine reads*, *Slowly*, *Keep as is* or *Leave out* | *Read again*, *Best of 3 reads*, *Read again slowly*, *Keep it anyway* or *Skip this track* | Match the buttons. |
| Recipes and steps | tile / tiles / Drive tile | step / steps / Drive step | *Tile* is a design word. The glossary says a tile is only how a step is drawn. |
| What it does table | as CHD or as dumped / transcoded (H.265 or AV1) | as CHD or as a plain image / made smaller (H.265 or AV1) | Match the step names. |
| Built-in recipes | keeps *Reset to built-in* | keeps *Reset to original* | Match the button. |

## Checked and kept

- **FLAC, MP3, Opus, AAC, ALAC, Vorbis, WavPack, WAV, CHD, M3U, cue sheet,
  ReplayGain, CD-Text:** names people meet in players and file lists. Each
  gets a plain description in the picker (section 1).
- **MusicBrainz, Discogs, GnuDB, TMDb, Redump, AccurateRip, LRCLIB,
  Cover Art Archive, MakeMKV, Jellyfin, ntfy:** proper names of real
  services. *Cover Art Archive* has to stay as Cover art's `.from`
  because a sentence uses it.
- **Drive, Audio CD, Data disc, Game disc, Video disc, Rip, Choose,
  Check, Folder, Next disc, Notify, Lyrics, Lyrics file, Cover art:**
  plain already.
- **Hold:** an everyday word. Only its description and statuses change.
- **Read again:** says what it does: up to two more careful reads, until
  two agree.
- **First read: fast / careful:** plain. Add a tooltip: "Careful is
  slower but checks each read as it goes."
- **Retries, Read speed, Drive's best:** everyday words for redumper's
  re-reads and speed.
- **Verified, Doesn't match, Damaged, Couldn't be read, Unverified:**
  plain verdicts. Only the details after them change.
- **Needs you, Waiting, Ready, Running, Done, Failed, Skipped, Not
  connected, Drive busy, Looking up, Not available, Ejected:** plain.
- **Synced / plain lyrics:** players call them that.
- **MakeMKV's beta key:** that's MakeMKV's own name for its free key,
  and the user sees it on MakeMKV's site.
- **Stopped, Didn't run, After {step}, default / yours / not set:** plain.
- **Disc ID:** shown as a reference number, the way MusicBrainz shows
  it.
- **Read offset:** kept, because AccurateRip and every ripping guide use
  it. It gets a tooltip and one label everywhere (section 3).
- **Personal access token, API key, Contact email:** the words Discogs,
  TMDb and GnuDB use on their own sign-up pages, so they match what
  people will see there.
- **Advanced section items** (Extra ffmpeg arguments, zstd/lzma/zlib,
  x265 presets, 10-bit, Tuned for): left as the tools name them. They're
  folded away for people who know them.
- **Edit, Save, Discard, Stay, Cancel, Delete, Duplicate, Undo, Redo,
  Start, Stop, Quit, Use this one, Use these, Enter details, Open
  folder, Expand, Restore:** plain.
- **Recipe names** (CDs, Data discs, Games, Films and TV, Archive
  everything, For the phone) and **folder names** on tiles (Library,
  Review, Phone): named by what they do, and editable.
