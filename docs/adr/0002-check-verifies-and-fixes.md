# 2. Check and fix: a game's image is verified and put right in one step

20 November 2026. Accepted.

## Context

Some drives misread data sectors without reporting it. On a real
PlayStation disc, each rip had about 200 sectors wrong, and they were in
different places each time. A copy can be put right in three ways, all
in Check since b9d2253 and 1f48c6e:

- fix bad sectors, by reading again the sectors that fail their own
  checks;
- rip the whole disc again, and combine the rips;
- compare with Redump's list, which alone can say the copy is the disc.

Steps see only their inputs, settings and outputs. So the question was
which step should do the reading that puts a copy right: Rip, which reads
the disc, or Check, which knows whether the result is right.

## Options

**A. Rip fixes itself; Check is a pure gate.**

- Rip finds the sectors that fail their own checks, re-reads them, and
  rips again and combines until every sector passes.
- Check only compares with Redump, and holds.

But passing every check isn't matching Redump. When the copy still
doesn't match, the next try needs Redump's answer. So Check would need
to read too, or ask Rip, and the line between them blurs.

**B. A feedback edge from Check to Rip.**

- Check is a pure function, `Dump → Verified | Request`.
- A request, such as *re-read these sectors* or *rip again*, goes back
  along a declared edge to Rip, the only step that reads.

This needs new engine machinery:

- a returned input on a step;
- a way to settle each item, so Rip knows when to stop waiting;
- care against deadlock, since Check finishes only after Rip, and Rip
  would wait on Check;
- a wire drawn backwards on the canvas.

**C, chosen. Check verifies and fixes, sharing the drive with Rip by its
lane.**

- Rip reads the disc once and hands on the raw image.
- *Check and fix* owns everything about how sound the image is: each
  sector's own checks, Fix bad sectors, ripping again and combining, and
  the Redump comparison.

## Decision

C, as built. The step's display name is *Check and fix*. Its id stays
`check`, so saved recipes and settings load as they are.

- **Least new machinery.** The engine already has the drive lane, so Rip
  and Check and fix never read at the same time. Nothing runs backwards.
- **Mostly built.** Fixing, ripping again, combining and comparing were
  already Check's. On the drive that misreads, it settled a disc by
  itself: 166 bad sectors read again and put in place, and the filed CHD
  is Redump's exact SHA-1 and MD5.
- **Clear lines.** Rip's sidebar shows what Rip's read reported. Check
  and fix's sidebar shows what it found, read again and put right, and
  whether Redump agrees.
- **Like a CD.** CD Rip reads and verifies in one step (AccurateRip,
  re-reads). Check and fix is the same kind of step for an image.

With *Try to fix it by itself* (the default), it fixes bad sectors, then
rips again and combines, up to *Extra rips*, then, last, repairs what
sectors it can by the disc's own error correction, stopping as soon as
the copy matches Redump. Then it asks. With *Ask me*, those actions are
buttons, the error correction last.

## Consequences

- Check and fix reads the disc. That's a deliberate exception to "a
  check only compares", recorded here. Every read goes through the drive
  lane, and none opens the tray.
- Verified still means one thing only. The final files, read again from
  disk, match Redump's entry on size, CRC-32, MD5 and SHA-1.
- **Repair by error correction is the last resort** (22 November 2026).
  It writes bytes no read gave back, so it comes after every way of
  reading the disc again, and only Redump can verify what it makes.
  Sectors that may be pressed bad on purpose (every read the same, or
  read once) are repaired only to see whether the copy then matches
  Redump, and put back as read if it doesn't. See `docs/audio-accuracy.md`,
  *Game and data discs*.
- **Not built yet:** re-reading the sectors that can't check themselves.
  These are the Mode 2 Form 2 sectors stored with an EDC of 0, which only
  their address checks, so a misread one passes every check.
  - The plan is three reads each, keeping the copy most reads agree on,
    and leaving alone any sector every read agrees with the image on.
  - It would come before ripping again, as quicker.
  - It would cover only those data sectors, never music. A music
    sector's place in the image depends on the offsets redumper applied,
    and nothing checks that a raw read lined up. Whole rips cover the
    music tracks.
