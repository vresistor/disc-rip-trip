# Audio accuracy

The rules the CD path follows. The code is in `crates/steps/src/real/`
(`mod.rs` for Rip, the drive and Folder, `cyanrip.rs`, `offset.rs`,
`encode.rs`, `cue.rs`, `fixtags.rs`); its tests hold each rule.

## The drive's read offset

- **The offset belongs to the drive model, not to a disc.** It is stored
  per drive and shared by every recipe that uses the drive.
- **It's found in this order:**
  1. AccurateRip's list of drive offsets, matched on the drive's maker
     and model. The list names some makers differently (Matshita is
     Panasonic, HL-DT-ST is LG Electronics, JLMS is Lite-ON).
  2. The offset the first app recorded for the drive, if any.
  3. Measuring, with a disc AccurateRip knows.
- **A measurement never overrides the list.** One disc can verify at more
  than one offset, because AccurateRip may hold several pressings of it a
  few samples apart. So *Measure* only counts for a drive the list
  doesn't have.
- **Rip won't start without an offset.** The Drive tile asks for it.

## Reading a track

- **One cyanrip process per track** (`-l N`). cyanrip 0.9.3 leaves a
  track's file empty until the whole process exits, so one process per
  disc can't hand tracks to the encoders as they finish. Retrying a track
  is the same call.
- **cyanrip does no lookups and no encoding here** (`-N -U -G`, WAV out).
  Lookups are steps; tags are written by ffmpeg from the chosen release.
  cyanrip 0.9.3 corrupts titles with a colon, so its tags are never used.
- **"Ripping errors: 0" on a fast read proves nothing.** A scratched disc
  can give a different checksum on every read with no errors reported.

## The samples are the disc's

- **Pre-emphasis is kept, not undone.** Some early CDs were mastered with
  pre-emphasis and say so in their TOC or subcode. cyanrip 0.9.3
  de-emphasises such a track by default, which changes every sample, so
  Rip passes `-W`. The flag is recorded instead, from Audio CD's look at
  the disc and from each read: the cue sheet says `FLAGS PRE` for that
  track, the rip log says *(pre-emphasis, kept)*, and the files carry
  `PREEMPHASIS=1` (a Vorbis comment, an ID3 `TXXX`, an MP4 freeform atom,
  an APE item; WAV has no place for it). A player or a later conversion
  that sees it can apply de-emphasis. If cyanrip ever says it
  de-emphasised a track, the read fails.
- **Gaps between tracks** go with the track before (cyanrip's default),
  so the files join with no gap and no overlap. Each track's file starts
  at its INDEX 01.
- **No overread.** With a positive offset, the last *offset* samples of
  the last track are past the end of what the drive reads, so they're
  silence (with a negative offset, the first samples of the first
  track). That is normal; AccurateRip ignores those samples.

## A hidden track before track 1

Some discs hide audio before track 1: track 1 starts later than the usual
two seconds and its pregap holds music.

- **It's found from the TOC.** Track 1 starting after sector 150 leaves
  `(start − 150) × 588` sample frames before it.
- **It's read once to look.** cyanrip drops track 1's pregap by default;
  `-p 1=track` splits it off as a track of its own, numbered 0, and
  `-l 0` reads it. Only track 0's read passes `-p`, so tracks 1 to N are
  read exactly as without it (cyanrip 0.9.3's source: the split leaves
  track 1's start and its AccurateRip checksums as they were).
- **Digital silence isn't kept.** If every sample is zero, nothing is
  filed and the log says so. (A drive that can't read the pregap and
  gives back zeros looks the same.)
- **Audio is track 0, "Hidden track"**: its own row, its own file
  (`00 - Hidden track.flac`), `TRACKNUMBER=0` of the disc's 13.
- **It must be exactly the TOC's length.** cyanrip's sample count and the
  WAV's both must equal it, or the read fails and the track waits for you.
- **AccurateRip holds nothing for it**, so it's settled by two reads that
  agree (*✓ two reads agree*), or by you, like any track without a match.
- **In the cue sheet** it is track 1's INDEX 00: its file opens `TRACK 01`
  with track 1's title, and track 1's file is `INDEX 01`. A cue sheet has
  no track 0.
- **Not yet tried on a disc that has one.** The above follows cyanrip
  0.9.3's source and is tested with a stand-in for cyanrip; reading a
  real hidden track on a real drive is still to be done.

## Verdicts

Each track's row says how it was settled. The note Rip records is on the
left, what the row shows beside it on the right:

| Note | Row shows | Means |
|---|---|---|
| *✓ 12* | *Verified 12* | The checksum matches 12 other rips in AccurateRip (its *confidence*). Proof; nothing more is read. |
| *✓ two reads agree* | *Reads agree* · no AccurateRip match | AccurateRip has no match (or the disc isn't in it), but two reads give the same checksum. |
| *✓ best of 5 reads* | *Best of 5* | Every sample settled by a majority of careful reads. Better than any one read, not proof. |
| *kept anyway, not verified* | *Kept anyway* · not verified | Kept by hand without proof. |
| *439 samples differ, 0:17–0:43* | *Damaged* · Reads differ at 0:17–0:43 (439 samples) | The reads don't agree: the disc is damaged there. Needs you. |

Runs from before November 2026 recorded *✓ combined from 5 reads* and
*kept, unverified*; they're still read the same way.

When the first read doesn't verify, Rip reads again carefully at once. A
track that still isn't settled waits for you, and its row offers:

- **Read again:** two more careful reads, stopping as soon as two agree.
  A scratch the drive reads differently every time won't converge, so it
  doesn't keep trying.
- **Best of 3 reads:** enough careful reads for a vote (three), then the
  majority of every sample. The reads that vote include the fast first
  read, not only the careful ones. A sample where no value has a majority
  keeps the most common one (the earliest read's on a tie); the row says
  how many samples were *uncertain*, and the track is filed as kept.
- **Read again slowly:** as *Read again*, at 4×.
- **Keep it anyway:** filed, marked not verified.
- **Skip this track:** not filed.

## Re-reads and the drive's cache

Some drives keep the audio they've just read in memory and hand it back
on the next read of the same sectors without going to the disc. A
re-read served that way agrees with the first read whatever the disc
holds, and *✓ two reads agree* would mean nothing. Checked on 20
November 2026 against cyanrip 0.9.3 and libcdio-paranoia 10.2+2.0.2 (the
versions `scripts/build-cyanrip.sh` builds), and on the MATSHITA BD-MLT
UJ240AS.

**What our reads do.** Rip's first read passes `-P 0` (unless the recipe
asks for careful), its re-reads `-P 3`. cyanrip turns 0 into
`PARANOIA_MODE_DISABLE` and 3 into `PARANOIA_MODE_FULL` without
`NEVERSKIP` (`src/cyanrip_main.c`, 104–109, set at 182). cyanrip itself
does nothing about the drive's cache: each read seeks to the track's
start and reads on (573). Its own repeat mode, `-Z`, which we don't use,
doesn't either (750–800).

**What paranoia does.** It keeps a model of the drive's cache, 1200
sectors (2.8 MB, 16 s of audio) unless told otherwise
(`lib/paranoia/p_block.h` 34, `p_block.c` 427–429; cyanrip never calls
`paranoia_cachemodel_size`). It reads that many sectors at a time
(`paranoia.c` 2669), and before each such read, at every level including
0, it checks whether the read starts inside what the drive may hold; if
so it reads one sector further back to make the drive seek, and times
it. A seek under 6 ms means the drive answered from memory, and paranoia
reports `PARANOIA_CB_CACHEERR` (`paranoia.c` 114, 2598–2621, called at
2738). That is how level 3's own re-reads inside one process stay
honest, as long as the drive's real cache is no bigger than the model.

**What it can't see.** The model starts empty in every process
(`cdcache_begin`/`cdcache_end` start at 9999999, `p_block.c` 427–428),
and every one of our reads is its own cyanrip process. So paranoia knows
nothing about what the previous read left in the drive. Between our
reads, only the order of reading protects us: a re-read starts at the
track's first sector, while the drive holds the last sectors the
previous read took, the track's end (and its read-ahead past it). By the
time the re-read reaches the end, the drive has refilled its memory from
the disc. That holds for any track longer than the drive's cache plus
its read-ahead. A track shorter than that could be handed back whole
from memory.

**What cyanrip shows.** paranoia's counts, at the end of each run's
output: `Paranoia status counts:` then lines like `READ: 336`, and
`CACHEERR: n` when there were any (`src/cyanrip_log.c` 327–353, counted
by `status_cb`, `cyanrip_main.c` 336–342). Rip doesn't read them yet.

**This drive.** `cd-paranoia -A` (Fedora's libcdio-paranoia
10.2+2.0.2, in a container): *Approximate random access cache size: 160
sector(s)*, *Drive cache tests as contiguous*, *Drive readahead past
read cursor: 24 sector(s)*, *Cache tail cursor tied to read cursor*, and
*Drive tests OK with Paranoia*. So it caches audio, 184 sectors at most
(2.5 s), well under paranoia's model. One thing wasn't tested: re-reading
the cached sectors was slower than reading the disc (15.67 ms a sector
against 0.76), so `-A` skipped its test that a backward seek empties the
cache (*Cache access insufficiently faster than media access*,
`src/cachetest.c` 785–787). Then track 3 (105 s) read twice with Rip's
exact arguments, `-P 0` then `-P 3`, offset +103: the same samples
(byte-identical WAVs, EAC CRC `FD03A15A`, AccurateRip v2 match at
confidence 30), no `CACHEERR` in either. Nor in the 13 first reads kept
from an earlier run.

**So, today.** With this drive, *two reads agree* is sound for every
track of 2.5 s or more, which is every real track: the Red Book's
shortest is 4 s (300 sectors). The exposed case is the hidden track
before track 1, which can be any length, and a drive with a bigger
cache: on a drive that holds, say, 10 s of audio, a short track's
re-read could be the drive's memory. Only this drive has been measured.

**What others do.** whipper reads every track twice, each in its own
`cd-paranoia` process at full paranoia, and compares CRCs
(`whipper/program/cdparanoia.py` 476–484, 526–535, at commit
`71251a0`), as we do. It runs `cd-paranoia -A` on request (`whipper
drive analyze`, `whipper/command/drive.py` 32–58), stores the result per
drive, and only writes it into the log as *Defeat audio cache*
(`cdparanoia.py` 583–626, `whipper/result/logger.py` 43); ripping is
the same either way. EAC is closed; its own pages say its feature test
detects a drive that caches audio, that the option must then be set,
and that the drive's cache is then flushed between reads, "in newer
versions" by reading sectors elsewhere, which makes secure mode much
slower ([feature detection](https://exactaudiocopy.de/automatic-feature-detection-technology/),
[extraction](https://exactaudiocopy.de/extraction-technology/)). That
is the vendor's description, not code we could read.

**Recommended fix** (not built yet):

1. **Read somewhere else first.** Before every re-read that will be
   compared, read 2400 sectors (twice paranoia's model, 5.6 MB) from a
   part of the disc at least that far from the track, and throw them
   away, over SG_IO as the TOC read already does; about two seconds at
   this drive's speed. It
   costs nothing on a drive that doesn't cache, and it doesn't depend
   on knowing the cache's size. This is EAC's approach.
2. **Read paranoia's counts.** Parse `CACHEERR` from each read's output
   and record it with the read. A re-read with any doesn't count towards
   *two reads agree*; the row says why.
3. **A test that catches it.** With a track shorter than the drive's
   cache (a hidden track of a second or two, or a drive with a large
   cache): read it, read it again at once, and time the second read; a
   read many times faster than the disc can turn means memory. Then do
   the same after (1) and check the time is the disc's. In the unit
   tests, a stand-in cyanrip that prints `CACHEERR: 3` must leave the
   track unsettled.

Until (1) is in, a hidden track of under 3 s settled by two reads that
agree should be taken with that in mind.

## Encoding

- **Nothing changes the samples** on the way to a lossless file: no
  resampling, no dither, no filters.
- **Every lossless file is checked.** After a FLAC, ALAC, WavPack or WAV
  is written (tags and all), it's decoded and its samples' MD5 compared
  with the rip's. A mismatch fails the track; it isn't filed. Lossy
  formats (MP3, Opus, AAC, Vorbis) change the audio by design and aren't.
- **Only the tags given are written**: nothing of the rip WAV's own.
- **Tags written after ffmpeg** (see below) never touch the audio: a
  lossless file is checked against the rip after them, and a lossy file's
  decoded audio must be the same before and after them.

## Tags

Each container's tags are where MusicBrainz Picard writes and reads them
(checked against Picard's `formats/` modules, and by reading files back):

| | Vorbis comments (FLAC, Opus, Vorbis) | ID3 (MP3) | MP4 (AAC, ALAC) | APEv2 (WavPack) |
|---|---|---|---|---|
| Track | `TRACKNUMBER=1`, `TRACKTOTAL=13` | `TRCK 1/13` | `trkn` | `Track 1/13` |
| Disc | `DISCNUMBER`, `DISCTOTAL` | `TPOS 1/2` | `disk` | `Disc 1/2` |
| ISRC | `ISRC` | `TSRC` | `----:ISRC` | `ISRC` |
| Label | `LABEL` | `TPUB` | `----:LABEL` | `Label` |
| Release | `MUSICBRAINZ_ALBUMID` | `TXXX:MusicBrainz Album Id` | `----:MusicBrainz Album Id` | `MUSICBRAINZ_ALBUMID` |
| Release group | `MUSICBRAINZ_RELEASEGROUPID` | `TXXX:MusicBrainz Release Group Id` | `----:MusicBrainz Release Group Id` | `MUSICBRAINZ_RELEASEGROUPID` |
| Release track | `MUSICBRAINZ_RELEASETRACKID` | `TXXX:MusicBrainz Release Track Id` | `----:MusicBrainz Release Track Id` | `MUSICBRAINZ_RELEASETRACKID` |
| Recording | `MUSICBRAINZ_TRACKID` | `UFID` (`http://musicbrainz.org`) | `----:MusicBrainz Track Id` | `MUSICBRAINZ_TRACKID` |

(`----:` is `----:com.apple.iTunes:`.) Barcode and catalogue number are
`BARCODE`/`CATALOGNUMBER` (`TXXX` in ID3, freeform in MP4, `Barcode`/
`CatalogNumber` in APE). ffmpeg writes most of these; it can't write the
`UFID` frame or MP4 freeform atoms, which are added afterwards with lofty,
reading the file without its conversions so nothing else changes, and
keeping the ID3 version chosen. A disc number comes from MusicBrainz (the
medium's position and the release's number of media). WAV has RIFF INFO
only: title, artist, album, date and track. Its `ISRC` chunk means
*source*, so the ISRC isn't written there.

## Filing

- **A Folder never files a track over a different file without asking**
  (tracks and the files that go beside them; DVD and Blu-ray titles and
  disc images are filed as before, over what's there). A file
  already there with the same bytes counts as filed. Different files with
  the same names wait at the Folder, everything before it carries on, and
  once the disc's files are all in, it asks once: *3 files already in
  Artist / Album*: **Replace them** (each copied in under a temporary name
  and renamed over the old one), **Keep both** (the new ones get " (2)",
  or the next free number; a numbered file with the same bytes counts as
  filed), or **Skip** (the new ones aren't filed and go no further). Keep
  both is picked by default, as it loses nothing.
- **What comes after a Folder sees the names it chose**: the cue sheet,
  Lyrics file and Notify use the files as filed.
- When two steps ask at once, the second waits until the first is
  answered.
- The cue sheet and rip log are named for the album and written beside
  the files by Cue sheet itself, not a Folder: a second rip of the same
  album replaces them.

## Files filed before 20 November 2026

Files filed before the audio audit carry two mistakes: cyanrip's name as a
comment (`comment=cyanrip 0.9.3`, in every format), and a cover of picture
type *Other* rather than *Front cover* (FLAC and MP3). To mend them:

    drt fix-tags ~/Music            # lists what it would change
    drt fix-tags ~/Music --apply    # changes it

It removes a comment whose whole value is `cyanrip <version>`, and only in
those files changes a lone picture of type Other to Front cover. Nothing
else. Each file is changed as a copy beside it, checked (the decoded
audio's MD5, every tag ffprobe sees, every tag lofty reads, pictures by
their bytes: all the same but for the change), and only then renamed over
the original. A file that fails a check is left as it was, and listed.

## Stopping

Stop keeps what was already finished: tracks read, encoded and filed
stay as they are.
