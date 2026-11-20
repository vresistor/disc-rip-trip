# Checking and repairing rips with the CUETools Database

20 November 2026. Proposed; nothing here is built yet.

## Why

AccurateRip checks each track on its own: a track's checksum matches other
people's rips of the same disc or it doesn't. When it doesn't, Rip settles
the track by two reads that agree, or by a vote of reads. Neither is proof.
A track the disc can't give back cleanly waits for you, and the best you
can do is keep it with its doubtful samples.

The CUETools Database (CTDB) holds a second kind of record for each disc
it knows: one checksum of the whole disc's audio, one per track, and *repair
data*. With repair data a rip that is wrong in a few places can be put
right, sample for sample, and then checked against the same record. It
answers the case AccurateRip can't: a damaged disc, or a pressing
AccurateRip doesn't hold.

This doc says how CTDB works, from its source, and how Rip would use it.

## Sources

- **CUETools**, the C# reference client: github.com/gchudov/cuetools.net
  at `f2ec334` (`CUETools.AccurateRip/AccurateRip.cs`, `CDRepair.cs`,
  `CUETools.CTDB/CUEToolsDB.cs`, `CUETools.CDImage/CDImage.cs`,
  `CUETools.Parity/`, `CUETools.Codecs/CRC/CRC32.cs`).
- **The server**, also public: github.com/gchudov/db.cue.tools at
  `47ec9a2` (`utils/docker/…/lookup2.php`, `submit2.php`,
  `phpctdb/ctdb.php`).
- **AccurateFix**, an independent C program that verifies and repairs:
  github.com/rdelien/accuratefix at `4c4b22f` (`docs/checksums.md`).
- **Our own rip**, checked against a live answer (below). XLD is closed
  and wasn't looked at.

Line numbers are for those commits.

## What CTDB holds

### The disc's audio, as one stream

CTDB sees a disc as one run of samples, from track 1's INDEX 01 to the end
of the last audio track: every track in order, each gap with the track
before it. Call its length *L* sample frames. That is exactly what Rip's
per-track WAVs make when put end to end, because cyanrip gives the gaps to
the track before and drops track 1's pregap
(`AccurateRip.cs` 406–413: the disc's length is `(AudioLength − Pregap) ×
588`). Checked: our 13 WAVs of the disc in the drive add up to 112,347,984
frames, the TOC's 191,068 sectors × 588.

The audio hidden before track 1 isn't part of it. Data tracks aren't
either. On a disc with a data track after the audio (an enhanced CD) the
audio ends 11,400 sectors before the data track starts (`CDImage.cs`
280–281).

### The checksums

- **CRC-32, the ordinary one** (zlib's: polynomial `0xEDB88320`, start and
  final `0xFFFFFFFF`, `CRC32.cs` 59–60, 120–129), over the samples as
  16-bit little-endian words, left then right (`AccurateRip.cs` 527–547).
- **Not over all of the stream.** The first 5,880 frames (10 sectors) are
  left out, and the last `5,880 + (L mod 5,880)` frames (`AccurateRip.cs`
  442–445 with `CDRepair.cs` 29–30; `CUEToolsDB.cs` 509 sets the stride to
  10 × 588 × 2 words). So a drive offset of up to 5,879 samples, and the
  samples it can't reach at the disc's ends, never change the checksum.
  With our +103, the 103 silent frames at the end are well inside the part
  left out.
- **One per track too** (`trackcrcs`): the same region cut at each INDEX
  01. Track 1's starts 5,880 frames in, the last track's stops where the
  disc's does (`AccurateRip.cs` 399–440). The disc's checksum is the
  tracks' run together.
- **No offset is stored.** The checksum is of whatever stream was
  submitted, corrected by its ripper's drive offset.

Checked against the live answer for the disc in our drive: from our rip's
WAVs (offset +103, every track an AccurateRip match), the disc checksum
and all 13 track checksums come out exactly as CTDB's top entry, but only
once our stream is moved by 12 samples: CTDB's audio sits 12 samples
earlier in its stream than in ours. That is a different pressing, which is
what the offset search below is for. It also confirms the 5,880 frames
left out at the start (track 1) and at the end (track 13).

### Looking a disc up

    GET https://db.cue.tools/lookup2.php?version=3&ctdb=1&fuzzy=0&metadata=none&toc=0:17809:37000:…:176016:191068

(`CUEToolsDB.cs` 74–83.)

- **`toc`:** each track's start sector, counted from 0 (MusicBrainz's
  numbers less 150), joined by colons, a data track's with `-` in front,
  then the lead-out (`CDImage.cs` 511–518). Cyanrip's MusicBrainz TOC
  gives it for an audio-only disc; a disc with a data track needs the
  TOC that marks it (`media.rs`'s `toc_starts` reads it).
- **`fuzzy=1`** also returns entries for the same audio with a different
  TOC (a data track, a hidden track): the server matches them on a hash of
  the audio tracks' starts (`ctdb.php` 27–42). Rip asks with `fuzzy=0`.
- **`metadata=none`:** CTDB can also return titles; we don't need them.
- **Not found** is HTTP 404 with the body `Not Found` (`lookup2.php`
  92–98).
- CUETools uses plain `http://db.cuetools.net`. **`https://db.cue.tools`**
  gives the same answer over HTTPS; `https://db.cuetools.net` fails its
  certificate check.

The answer for the disc in our drive (20 November 2026, trimmed; the same
three entries with `fuzzy=0` and `fuzzy=1`):

```xml
<ctdb xmlns="http://db.cuetools.net/ns/mmd-1.0#" xmlns:ext="http://db.cuetools.net/ns/ext-1.0#">
 <entry confidence="283" crc32="692a5723" hasparity="http://p.cuetools.net/3824991" id="3824991"
        npar="8" stride="5880" syndrome="1ZDkAZ8spb+Y2mGq3p351w=="
        toc="0:17809:37000:44875:53025:66628:75925:92340:121160:136635:149200:163135:176016:191068"
        trackcrcs="548cc2c3 178087a9 3025d3f3 69d381cc 80c2b6b9 f9c51fa5 7cfaa988 aa4f71e9 4664ad89 bf1e18ed 0cfbb405 9fcbea69 f348fcf8" />
 <entry confidence="1" crc32="c865c210" id="5573536" npar="8" parity="D1cbuVLUz7WcgORzClj9kA==" stride="5880" toc="…" trackcrcs="379353af …" />
 <entry confidence="1" crc32="8b52afc2" id="9970106" npar="16" stride="5880" syndrome="1ZDkAZ8spb+Y2mGq3p3515FRro2iMHE6OTRQ/XJpa+Y="
        toc="…" trackcrcs="548cc2c3 … 4664ad89 b6eae719 56d2807c dd7554ac f348fcf8" />
 <metadata />
</ctdb>
```

Each `entry` is one version of the disc's audio (`lookup2.php` 116–127):

| Attribute | What it is |
|---|---|
| `id` | the entry |
| `confidence` | how many rips were submitted that match it |
| `crc32`, `trackcrcs` | the checksums above, in hex |
| `npar` | how much repair data it has: 8 or 16 |
| `stride` | always 5,880 frames |
| `syndrome` | the first column of the repair data (base64), to check the download against |
| `hasparity` | where the repair data is; missing on old entries, which carry only `parity` and can confirm a rip but not repair it |

The third entry is the same audio as the first but for tracks 10–12: one
rip, most likely of a damaged copy.

### Different pressings: the offset search

The same album can be pressed with the audio a few samples earlier or
later. CUETools finds an entry's alignment from the repair data, not the
checksum (`CDRepair.cs` 103–157): for every shift from −5,879 to +5,879
samples it compares the entry's first column of repair data with the
rip's. An exact match gives the shift; failing that, the shift whose
difference decodes to the fewest errors (fewer than `npar / 2`). The
checksums are then taken at that shift. Entries whose audio length or
stride differ from the disc are passed over (`CUEToolsDB.cs` 442–447).

**Confidence.** CUETools reports the disc as matching *N* rips, the sum of
`confidence` over entries with this exact TOC that match without errors
(`CUEToolsDB.cs` 479–491). A track can match an entry the disc as a whole
doesn't, through its own checksum (`CUEToolsDB.cs` 549–591).

### Repair data

- **What it is:** Reed–Solomon over 16-bit symbols (GF(2^16), polynomial
  `0x1100B`, `Galois.cs` 552). The stream, less the parts left out at
  each end, is cut into rows of 11,760 words (10 sectors); each of the
  11,760 columns, one word from every row, is protected by its own code
  (`CDRepair.cs` 29–31, `AccurateRip.cs` 493–496).
- **The file** at `hasparity` holds `npar` × 11,760 little-endian 16-bit
  words, syndromes for each column: 188,160 bytes for npar 8 (fetched
  for the entry above), 376,320 for npar 16. The first `npar × 11,760 × 2`
  bytes of a bigger file are the smaller one, so CUETools fetches 4, then
  8, then 16 with byte ranges as it needs them (`CUEToolsDB.cs` 200–212,
  457–473), and checks column 0 against the lookup's `syndrome` before
  using it (`CUEToolsDB.cs` 255–260).
- **Where:** `http://p.cuetools.net/<id>`, plain HTTP only (the HTTPS
  name resets the connection). The same file comes over HTTPS from
  `https://s3.amazonaws.com/p.cuetools.net/<id>`, byte for byte (checked
  for 3824991), but that address is nowhere documented.
- **How repair works** (`CDRepair.cs` 159–245): for each column, the rip's
  syndromes against the entry's; where they differ, the standard decoding
  (Berlekamp–Massey, Chien search, Forney) gives which words are wrong and
  what they should be. A correction outside the audio fails the repair.
  The fixed stream's checksum must then equal the entry's `crc32`.
- **How much it can fix:** up to `npar / 2` wrong words in each column. The
  decoder doesn't use known positions of bad samples (`RsDecode.cs` has no
  erasure path). One stretch of damage up to `npar / 2 × 10` sectors long
  fits: 40 sectors (0.53 s) with npar 8, 80 (1.07 s) with npar 16. Damage
  spread over the disc counts towards the same columns, so two scratches
  in different tracks share the budget. For scale: *439 samples differ*
  is 878 words, under one word per column.

### Whole disc only

The disc checksum and the repair data are of the whole stream. CUETools
won't search for an offset until every sample has been seen
(`CDRepair.cs` 110–111). A single track can be compared on its own
checksum, but at a shift other than 0 its window takes up to 5,879
samples from each neighbouring track. Nothing can be repaired before the
last track is read.

### Submitting

`POST …/submit2.php`, multipart: TOC, checksums, the first column of
repair data, a hashed machine id, a quality score, AccurateRip's
confidence, optionally the drive (`CUEToolsDB.cs` 346–400). The server
asks for the full repair file when it wants it, refuses rips that score
under 50, refuses a client that hasn't seen the newest entries, and
refuses drives it doesn't know (`submit2.php` 132–194; the list of drives
isn't public). It stores the user agent, IP address, machine id and
drive. Other rippers submit (its public statistics list several), so
there's no list of allowed programs in practice, but there is no written
policy on it either.

## Design

### When Rip asks CTDB

1. **Before reading, a lookup.** As soon as Rip starts, it looks the disc
   up (`fuzzy=0`) in the background. Nothing waits on it. No entries, an
   error, or no network: Rip goes on exactly as today.
2. **Reading is as today.** A track AccurateRip matches is proof and goes
   on to the encoders at once.
3. **Tracks without that proof wait for the disc.** If CTDB holds the
   disc, a track settled by two reads that agree, or not settled at all,
   isn't sent on until every track has been read once. Its row says
   *waiting for the rest of the disc*. Without CTDB entries, they go on
   at once as today.
4. **After the last track: the whole disc.** Rip runs over the tracks'
   WAVs in order (each track's chosen read: the matching one, else the
   last careful read), working out the checksums and syndromes as it
   goes. It doesn't hold the stream in memory: one pass over the files,
   keeping 11,760 × npar syndromes (376 KB for npar 16) and the CRC
   state. Then it finds each entry's shift, and for each waiting track:
   - **its samples are all covered by matching checksums** at an
     entry's shift: verified by CTDB, sent on. At a shift other than 0
     a track's checksum doesn't line up with its file: at +12, track N's
     checksum leaves out its own first 12 samples and takes track N+1's
     first 12. So a track counts when its own checksum matches and so
     does the neighbour's on the side its file reaches into (track N−1
     for a positive shift, N+1 for a negative one), or when the disc's
     `crc32` matches;
   - **it's damaged** (reads disagree, or two agree but CTDB disagrees)
     **and an entry has repair data:** Rip fetches it and repairs. A
     repair counts only if (a) every correction falls in tracks that
     aren't already proven (a correction inside an AccurateRip match
     means the entry isn't our audio: no repair), (b) the repaired disc's
     checksum equals the entry's `crc32`, and (c) each repaired track
     passes the rule above. Then the repaired WAV is the
     track's file, and it's sent on;
   - **neither:** as today. *Two reads agree* still settles a track, and
     a damaged track waits for you with the usual choices.
5. **After *Read again* or *Best of 3 reads*,** Rip checks the disc again
   with the new read, so a vote that leaves a few samples uncertain can
   still end matched or repaired.

**Which entry.** For checking: any entry, the row giving its confidence.
For repair: the entry with the most confidence among those with repair
data that the disc's proven tracks agree with. An entry of confidence 1
can still repair, but the row says 1.

**The disc's two ends** are in no CTDB checksum and can't be repaired:
track 1's first 5,880 samples (plus the shift) and the last track's final
5,880 to 11,759. (AccurateRip leaves out 2,939 at the start and 2,940 at the end.)
So for track 1 and the last track, CTDB's word counts only if the reads
also agree on those samples, compared sample by sample; if they don't,
the track is as unsettled as before, and the row says where.

**Tracks to rip other than *all*:** no CTDB. It needs the whole disc; the
log says so once. **The hidden track** isn't in CTDB's stream; it's
settled as today.

### What the notes and rows say

New notes, in the style of the table in `audio-accuracy.md`:

| Note | Row shows | Means |
|---|---|---|
| *✓ 283 in CTDB* | *Verified 283* · in CTDB | AccurateRip had no match, but the track's checksum matches 283 rips in CTDB. Proof. |
| *✓ repaired, 439 samples, 0:17–0:43 (CTDB 283)* | *Repaired* · 439 samples at 0:17–0:43, from CTDB | The reads didn't agree there. CTDB's repair data put those samples right, and the whole disc now matches 283 rips. Proof. |
| *439 samples differ, 0:17–0:43; too much damage for CTDB* | *Damaged* · Reads differ at 0:17–0:43 (439 samples); too much for CTDB to repair | Needs you, as now. |
| *439 samples differ, 0:17–0:43; CTDB has no repair data* | *Damaged* · …; CTDB can't repair this disc | Needs you. |

- **The disc's line** in the rip log: *CTDB: the disc matches 283 rips
  (entry 3824991), 12 samples earlier*; or *CTDB: not in the database*;
  or *CTDB: couldn't ask (no answer)*.
- **A track AccurateRip matched** keeps *✓ 12*; if CTDB agrees, the log
  says so, and the row doesn't change.
- **`is_accurate`** (only "✓ digits" counts as proof today) learns the
  two new notes, so the rip log's count of verified tracks, the cue
  sheet's log (`cue.rs` 154) and Notify count them. *✓ two reads agree*
  still doesn't count.
- The rip log says which: AccurateRip, CTDB, or repaired.

### Where the code goes

- `crates/steps/src/real/ctdb/mod.rs`: the lookup and the repair file
  (`net::client`, which is HTTPS-only; XML with `quick-xml`, already a
  dependency), the TOC string, and the choice of entry.
- `ctdb/stream.rs`: the disc as one stream over the WAVs, the parts left
  out at each end, the checksums at a shift (`crc32fast`'s `combine`, as
  CUETools does).
- `ctdb/repair.rs`: GF(2^16), syndromes, the offset search, decoding and
  writing the repaired WAV. Pure functions, no I/O but reading samples.
- `mod.rs` (Rip): steps 1, 3, 4 and 5 above, and the notes.
- **Network:** the lookup over `https://db.cue.tools`. The repair file
  only from the HTTPS S3 address; if that ever stops working, plain HTTP
  from `p.cuetools.net` is safe enough to fall back to, since the file is
  checked against the lookup (which came over HTTPS) and the repaired
  disc against the entry's checksums, but it needs `net` to allow one
  plain-HTTP address, as Redump does. The README's network table gets a
  CTDB row; what's sent is the TOC, nothing else.
- **Later:** a `drt ctdb <run folder>` that checks a finished rip, which
  is also how to try it on real discs before Rip uses it.

### Submitting: not at first

Rip doesn't submit. A submission puts our audio in front of everyone who
rips the disc after us, so a wrong one costs others; the server wants a
quality score and a drive name from its own list, neither of which we can
check without submitting for real; and it stores the IP address and a
machine id, which an app that may be published shouldn't send without
asking. If it's added later: only discs where every track matched
AccurateRip at confidence 2 or more and CTDB has no matching entry
(CUETools' own rule for files, `frmCUETools.cs` 1058–1068), only when you've switched it on, with the
drive name left out.

## Tests

- **The real answer as a fixture.** The XML above, whole, and the 188,160
  bytes of repair data for entry 3824991, in `crates/steps/testdata/ctdb/`.
  Parsing gives three entries; folding each entry's track checksums gives
  its `crc32` (true for all 484 entries on the four discs looked
  at); the repair file's column 0 equals the `syndrome`
  attribute.
- **The geometry, from the TOC:** L, the 5,880 frames at the start, the
  10,584 at the end for this disc (5,880 + 4,704), track 1's window, the
  last track's, and an enhanced CD's 11,400 sectors.
- **Our rip of this disc,** as an ignored test that reads WAVs from a run
  folder (they're 450 MB, too big for the repo): the shift comes out as
  12, the 13 track checksums and the disc's match entry 3824991. The same
  test then changes 400 samples of one track in a copy, repairs it from
  the fixture's repair data, and expects the original back, byte for
  byte, and the note *✓ repaired, 400 samples*.
- **Synthetic discs, small:** a stream of a few thousand sectors of
  generated audio; its repair data made by our own encoder; damage of
  `npar / 2` sectors in one stretch is repaired, `npar / 2 + 1` in the
  same columns is refused; a correction landing in an AccurateRip-proven
  track is refused; damage at a shift of ±5,879 repairs.
- **Rip, with the stand-in cyanrip and a stand-in server:** a track
  without an AccurateRip match waits for the disc and ends *✓ 283 in
  CTDB*; a damaged one ends repaired; no network leaves everything as
  today; *tracks 3, 5* never asks CTDB.

## Risks and what isn't known

- **Repairing the wrong audio.** A remaster or a different mix with the
  same TOC could pass a lax check. The three conditions in step 4, and
  never correcting a proven track, are the guard; CRC-32 isn't
  cryptographic, so the per-track checksums are checked too, as
  AccurateFix does.
- **Our decoder must be right.** It's Reed–Solomon written from CUETools'
  source. The ignored test on a real disc and real repair data is the one
  that counts; synthetic tests made with our own encoder can't catch a
  mistake shared by both.
- **The shift's sign.** CUETools' code turns it round twice
  (`CUEToolsDB.cs` 355, 574). Our test fixes it: CTDB's audio is 12
  samples earlier than ours on this disc.
- **Why 12.** Our rip matched AccurateRip at confidence 30, while the
  same logs say AccurateRip's best for the disc is 67–68 (*max
  confidence*). So AccurateRip holds a more common pressing too, most
  likely the one CTDB's main entry is from, 12 samples from ours. Likely,
  not shown.
- **The service.** One person runs it. The HTTPS lookup name and the S3
  address aren't what CUETools uses and could change.
- **Discs with data tracks** need the TOC with the data track marked and
  the 11,400-sector rule; not tried on a real one.
- **Not yet tried** on a disc that needs repair. The repair path is
  described from source and AccurateFix's documentation; none of it has
  been run here.

## The drive's cache

Whether a drive's cache can make *two reads agree* meaningless was looked
into at the same time. It's in `audio-accuracy.md`, *Re-reads and the
drive's cache*: a hidden track of under 2.5 s, or a drive with a bigger
cache, could have been fooled, so since 20 November 2026 Rip reads 2,400
sectors from somewhere else on the disc before each re-read it compares.
