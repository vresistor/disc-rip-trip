# Trip

Turning physical discs into files in a library, by running each disc
through a recipe of steps.

## Discs and drives

**Disc**:
The physical thing in a drive. Has a disc kind: audio CD, DVD, Blu-ray,
game disc, or data disc.
_Avoid_: Media, CD (unless it is one)

**Drive**:
An optical drive. Owns the properties that belong to the hardware rather
than to any disc, such as its read offset.

**Read offset**:
How many samples early or late a drive reads audio. A property of the
drive, needed before any audio read can be checked.
_Avoid_: Offset (alone), drive correction

## Recipes and runs

**Recipe**:
A named pipeline of steps that you make, starting at one drive, that says
what happens to one kind of disc in that drive.
_Avoid_: Workflow, mode, preset, profile, lane

**Step**:
One stage of a recipe. Takes items of one kind and gives items of one kind,
and is offered only where what it needs has been found before it.
_Avoid_: Node, stage, phase, tile (a tile is how a step is drawn)

**Run**:
One disc going through one recipe, from insert to the last step.
_Avoid_: Rip, job, session

**Kind step**:
The step after the source that establishes what kind of disc this is
(audio CD, DVD, game disc) and lists its items without reading them. It
decides what every later step can be.
_Avoid_: Read as, scan, detect

**Rip**:
The step that reads items off the disc into files: a CD's tracks, a
DVD's videos, or a data or game disc copied whole to an image.
_Avoid_: Dump (except as Redump's own word), extract, read

**Item**:
The unit a step works on: a disc, a track, a video, a dump, or a file made
from one.
_Avoid_: Work item, entry

**Item kind**:
What a step takes or gives: a disc (before and after its kind step says
what it is), tracks, videos, a dump, or a file of one kind (audio, image,
video, text, pictures). *Any file* is what a step that takes every kind of file
takes, such as Folder.
_Avoid_: Type (alone), format

**Hold**:
A step that keeps the files reaching it until you let them through. Only
that disc waits; the drive and other branches carry on.
_Avoid_: Gate, pause, review

**Folder**:
A step that puts the files arriving from its left at its path, named by
its template. After another folder, it copies from that one.
_Avoid_: Library (the name of a folder tile, not a kind of step), output

**Next disc**:
A step that ends a disc: once it's read, it opens the tray and the recipe
starts again for the next one.
_Avoid_: Eject, loop, repeat

**Notify**:
The step that ends a recipe by saying how the disc went, on this computer
or through ntfy.
_Avoid_: Tell me, alert

**Sidebar**:
The panel on the right of the window for the step you clicked: its
progress, settings and log, and any question it's asking.
_Avoid_: Drawer, inspector, panel

## Metadata

**Identity**:
What a disc is, as the outside world knows it: an album release, a film,
a season of a show, a game release.
_Avoid_: Title (ambiguous with a DVD title), release (music only), match

**Part**:
One piece of an identity that an item can correspond to: a track of a
release, an episode of a season, the feature of a film.
_Avoid_: Track (when meaning the identity's side), episode (in general)

**Fact**:
One piece of metadata about an identity or a part, together with the
source it came from: an album title, a track's artist, a cover image, an
ISRC, a year.
_Avoid_: Tag (a tag is a fact written into a file), field, attribute

**Source**:
Anywhere facts come from: a catalogue (MusicBrainz, Discogs, Cover Art
Archive, TMDb, Redump), the disc itself (CD-Text, ISRCs, a volume label,
a game serial), or a file beside the rip.
_Avoid_: Provider, service, database

**Key**:
Something a source can be asked with: a disc ID, a table of contents,
ISRCs, a barcode, a serial, a hash, a volume label, a typed name. A fact
from one source can be the key for another.
_Avoid_: Query, ID (alone)

**Candidate**:
One identity a source offers as what the disc might be.
_Avoid_: Result, hit, suggestion

**Lookup**:
A step that asks one source for candidates or facts. A recipe has as many
as it has sources, each straight after what it looks up.
_Avoid_: Identify, names, metadata step

**Choose**:
The step that settles which candidate is the identity, from everything
the lookups before it found, takes one value per fact, and matches items
to the identity's parts. Asks you only when it isn't sure.
_Avoid_: Pick, resolve, confirm, identify

**Match**:
The pairing of each item with a part of the identity: track 3 with the
release's third track, video 2 with episode 9.
_Avoid_: Assignment, mapping, role

## Checking

**Check**:
Comparing what was read against a record of known-good reads of the same
disc, to say whether the read is right. Answers "did we read it
correctly?", where a lookup answers "what is it?".
_Avoid_: Verify (as a noun), validation

**Check and fix**:
The step after a game's or data disc's Rip. It checks the image (each
sector's own checks, then Redump's list) and, for a CD image, puts it
right by reading bad sectors again or ripping again and combining. Its
id is `check`. See `docs/adr/0002-check-verifies-and-fixes.md`.

**Verdict**:
The outcome of a check for one item: *Verified* (it matches N other rips
in AccurateRip), *Reads agree* (two reads are the same, with no
AccurateRip match), *Best of N* (each sample as most reads gave it),
*Kept anyway* (not verified), or *Doesn't match*.
_Avoid_: Status, result, consistent, combined, confidence (say *matches
N other rips*)

**Video**:
One video on a DVD or Blu-ray, a film, an episode or an extra: what the
disc format calls a *title*. *Title* is kept for a song's or a film's
name.
_Avoid_: Title (for a video on the disc)

**Skipped**:
An item you chose not to keep (*Skip this track*), or one a step went on
without.
_Avoid_: Left out
