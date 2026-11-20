# Design: the pipeline

The master design for the new app (`crates/`, `app/`). First written 18
November 2026 as a proposal; revised 19 November 2026 to match what is
built. Words are defined in `GLOSSARY.md`. The audio rules are in
`docs/audio-accuracy.md`.

Mockup: `docs/ux/mockups.html`. Open it in a browser; the buttons along
the top switch between states of the one view. The window now goes
further than the mockup in places; where they differ, this document and
the window are right.

## The idea

- **A recipe is a named pipeline of steps.** A drive, then what kind of
  disc it is, then whatever you want done, then where the results go.
  You make one with +, name it, and it's in the list down the side.
- **The running recipe is the interface.** Pick a recipe in the list and
  its pipeline fills the window, live, on a canvas. There are no screens
  per kind of disc, so any recipe you can build gets a usable interface
  without anyone designing one.
- **Configuration lives on the tile it affects.** The drive's read offset
  is on the drive's tile; a library folder is on its folder tile. There is
  no settings page. Anything unset makes its tile ask before the recipe
  can start.
- **The order reads like the job.** The disc's kind is established first
  (*Audio CD*: 17 tracks, from the table of contents), then each source is
  looked up as its own step, then one *Choose* step settles what the disc
  is, and only then is it ripped.
- **The code is the same shape.** A step is one small module behind one
  trait; a recipe is a tree of them. A new feature is a new step.

## What you see

Down the left, **the list of recipes**, each with its name, its drive,
and one line of status with an icon: *6 of 17*, *Which album is this?*,
*Not connected*, *Ready*. That list is how you see every drive at once;
only the one you pick is drawn. Discs still going or holding from an
earlier run are listed under their recipe, one line each.

On the right, the picked recipe. Its header is one line: the recipe's
name, the disc and drive, the overall status (*about 4 min left*,
*Needs you*), *Edit*, and *Start* or *Stop*.

Below the header is the recipe, on a canvas that scrolls both ways:

- **Tiles snap together left to right into a strip.** A strip reads like
  a sentence: *Drive, Audio CD*, then *MusicBrainz* and *CD-Text* side by
  side, then *Choose, Cover art, Rip*.
- **Lines appear only where a recipe splits or joins.** After Rip, the
  tracks go to a FLAC strip and an MP3 strip at the same time.
- **Every tile always shows its status.** A tile is a 288 × 192 card
  with the same five slots, each meaning the same thing on every card
  (`docs/ux/cards.md`):
  1. its name, with a glyph for its state;
  2. up to two of its settings, as label and value;
  3. the outcome, in the job's unit (*16/17 verified*, *123 MB*);
  4. one line of detail: what it's on now, or the most useful fact
     about the result;
  5. an item strip: one cell per track, title or file, and the only
     progress bar.

  State shows by the glyph and the outline, not a tint: busy, asking
  and failed cards are outlined. Orange only ever means *needs you*.
- **Steps the disc hasn't reached yet are blueprints:** dashed,
  see-through, name and settings only, with faint dashed wires. A step
  turns solid when it starts, so a run shows how far through the plan
  it is.

| State | Icon | Status line |
|---|---|---|
| Not started | empty circle, drawn as a blueprint | empty |
| Working | spinner, and the bar | blue: *6 of 17* |
| Done | green tick | *1999*, *Found* |
| Needs you | amber !, amber outline, its sidebar opens | amber: *Which album is this?*, *Offset?* |
| Waiting on the step before | clock | grey: *Waits for choose* |
| A problem with some items | red ✕ | red: *16 ✓ 1 ✕* |
| Holding until you let it through | pause | amber: *Holding*, then *All here* |
| Stopped before it ran | empty circle | grey: says it didn't run |

### The drive

The window watches the drives every 2 seconds. A recipe whose drive
isn't connected says *Not connected*, calmly, in grey: on its line in the
list, in the header, and on its Drive tile. Start is disabled until the
drive is back, and then the recipe is ready again by itself, with
anything that was waiting on the drive (its offset) looked up at once.
Another recipe reading from the same drive makes this one's Drive tile
say *Busy*, with that recipe's name.

The Drive tile's sidebar has a dropdown of every drive: the one this
recipe uses, and beside each whether it's busy with another recipe or
not connected.

## The window

Designed for **1280 × 720** (a 1080p laptop at 150%), and checked at
1600 × 900 and 1920 × 1080.

- **The list is 232 px; the canvas gets the rest.** The canvas scrolls
  both ways, and has zoom and *Fit to window*. Tiles are always 288 × 192 at 100%, so
  a recipe looks the same in any window; a bigger window shows more of
  it.
- **Zoom is only zoom.** *Fit to window* scales the whole canvas down; tiles keep
  the same layout and text, just smaller.
- **The sidebar** opens on the right: about a third of the window
  (440–680 px), wider for *Which album is this?*. The canvas keeps its full
  height and scrolls the tile it belongs to into view.
- **The window draws its own frame**: the top band drags it, and the
  window controls sit at the right of the header.
- **Light and dark** follow the system at start; the switch at the
  bottom of the list changes it.

### Motion

Motion says what changed; nothing moves while the window is idle.

- The camera glides to *Fit to window*, a zoom, or a tile scrolled into view. A
  drag or a scroll takes over from a glide.
- The sidebar slides in from the right (180 ms). Switching tiles with
  it open doesn't.
- A tile that finishes, fails, or begins to ask plays a one-shot cue:
  its tint comes in strong, then settles. A question also rings once.
- Wires into a busy tile carry dashes moving toward it; the spinner
  turns; a tick pops when a step finishes; buttons answer hover and
  press, and keyboard focus shows as a ring.
- It redraws every 33 ms while something moves (250 ms unfocused), and
  not at all when nothing does.

GNOME's *Reduce animations* (read once at start) or
`DRT_REDUCED_MOTION=1` turns every slide, glide and cue into an instant
change. Colours still change.

### The most complex recipe

The built-in *Archive everything* is the recipe we design for: four
lookups side by side, two lookups after Choose, three outputs, a branch
inside a branch (Library to NAS, Backup, a cue sheet and `.lrc` files
beside it), a line for the phone (Opus, ReplayGain, Phone), and one
step joining all the outputs at the end (*Notify*). (It had the phone's
line in a named group; the editor can't make groups yet, so it doesn't.) At 1280 × 720,
*Fit to window* shows all of it, small.

Its layout is the same tree as the engine's: a sequence lays its parts
left to right, branches stack top to bottom, a group draws a dashed box
with its name, and wires run from whatever ends one part to whatever
starts the next.

## Configuration is on the tiles

Clicking a tile opens its sidebar: its settings, and what it did.
That's the only place they're set.

| Tile | Its settings |
|---|---|
| Drive | which drive; read offset, with *Measure* (only in a recipe that rips audio) |
| Audio CD, Data disc, Game disc | nothing; they read the disc |
| Rip | *Ripping speed* (fast, or slow and careful), *Tracks to rip* (every track is checked against AccurateRip) |
| Discogs, GnuDB, TMDb | a token, contact email or API key, for the whole app |
| FLAC, MP3, … | the format's settings (see *Encoders*) |
| Folder | the folder, and the file names as a template with presets |
| Notify | *When*, *On this computer*, *Phone alerts (ntfy topic)* and its server |

**Some settings belong to a thing, not a recipe.** The read offset
belongs to the drive, so it's stored per drive and every recipe's Drive
tile shows the same value. Keys and tokens belong to the app, so they're
set once. The tile says so.

**Unset settings are resolved on the tile before the recipe starts,**
and whatever can be looked up is looked up the moment it can be. A
drive's offset is found as soon as a recipe has that drive (see
`docs/audio-accuracy.md`). Only when that fails is the tile amber,
asking for the offset with *Measure*. Anything else unset, like a
library folder, makes its tile amber and keeps Start disabled.
First-time setup isn't a screen; it's the tiles that need something
asking for it.

Every value records where it came from: *default*, *yours* (with the
date), *measured*, *AccurateRip, 528 agree*. That shows beside the value,
with *Reset* for one you changed. A step's settings are locked while it
runs.

## The sidebar

There's one sidebar, for one step at a time. It opens when you click a
tile, when a step needs you (its question), and when a tile needs
setting up (at the field that's missing). ✕, Esc, or clicking the tile
again closes it.

Its header has the step's name and state, one sentence, and the one
thing to do (*Use this one*, *Let 17 through*, *Open folder*). Below it
are up to three tabs, only the ones with something in them:

- **Progress:** what the step did, item by item, each with its proof.
  Rip shows a ring of the disc's tracks, the track it's on with the
  time left and the speed, then each track's AccurateRip result.
- **Settings:** one per row, locked while the step runs.
- **Log:** its lines, and its tool's own output.

It opens on Settings before a step has run or when a value is asked
for, on Log when the step failed, and on Progress otherwise. A step
shows only what it found or did itself; what other steps found meets at
Choose. Each recipe's last run is kept while the app is open. See
[docs/ux/step-record.md](docs/ux/step-record.md).

### When a step needs you

A step that needs you opens its sidebar, and its tile is outlined. The
best answer is already picked, so it's one click. The recipe's line in
the list says *Which album is this?* in amber, so you see it from any recipe.

Steps after the one asking wait for it; steps before it, and other
recipes, carry on. For a CD that means the drive waits while Choose
asks. Usually nothing asks: a step that can choose for itself does
(MusicBrainz with an exact disc-ID match, Redump with a matching hash).

Questions come in three shapes:

- **Pick one** of a few candidates (which album, which game), with
  *Enter details* to type it instead, and *Skip*.
- **A value:** a folder, an offset, a name.
- **A choice per item:** which episode each DVD title is, or skip it.

### Items

Every step works on items (tracks, titles, a dump, files), so every
sidebar lists them the same way: the item, its length or size, its state,
its verdict, and the retries the step offers. Filters across the top
(*Needs you*, *Verified*, *All*). A step that writes to a folder has
*Open folder*.

| Step | Retries |
|---|---|
| Rip (audio) | Read again, Best of 3 reads, Read again slowly, Keep it anyway, Skip this track |
| Rip (data disc), Rip (game) | Re-read bad parts, Keep it anyway, Skip this disc (not for a game) |
| Rip (video) | Read again, Skip this video |

## Editing a recipe

- **A + on a strip is how you add a step.** It opens a popover right
  there: a search box, then a list grouped by meaning (Look up, Decide,
  Rip, Check, Make, Save to, Notify), each with its icon and what it does. Arrow
  keys and Enter work. A + where the line splits adds a new branch
  beside the others; anywhere else it continues the line. A + between
  two tiles inserts there. Where branches come together there's a + too,
  on the wires where they meet (or, with nothing after them yet, where
  lines drawn in from each meet): what's added there waits for all of
  them and gets everything they found. A step that fits both in line and
  beside what follows has a small *Beside* (or *In line*) button on its
  row for the other way, so two Folders can sit side by side. There's no
  drag and drop.
- **Every built-in recipe can be made this way**, from its first step,
  one + at a time; a test rebuilds each one so.
- **The + offers only what can go there** (see *What follows what*). A +
  that would offer nothing isn't drawn.
- **Click a tile to change its settings**, or to remove it. The sidebar
  says what else would go (*Remove 3?*) and those tiles are marked first.
- **Editing is a draft.** Undo and redo (Ctrl+Z, Ctrl+Shift+Z); *Save*
  keeps it, *Cancel* drops it, and leaving with unsaved changes asks. A
  recipe with an unsaved draft can't start. Settings changed outside
  editing are kept at once.
- **The header renames, duplicates and deletes** the recipe while
  editing.
- **A group of tiles acts as one step;** its output is its last tile's.
- **Built-in recipes are editable,** and each keeps *Reset to original*.
- **A step this build doesn't have** (from a newer recipes file) shows as
  a muted tile, *Not available*, and can be removed. The recipe can't
  start until it is.

## Steps

### Typed, with a context

Every step takes items of one kind and gives items of one kind. The
kinds:

- *nothing* (what Drive takes, and what Notify and Next disc give);
- *disc*, then *audio disc*, *data disc*, *game disc* or *video disc*
  once a kind step has looked;
- *tracks*, *titles*, a *dump*;
- files: *audio file*, *image file*, *video file*, *text file*, and *any
  file* for a step that takes any (Folder, Hold);
- *anything*, for a step that takes whatever arrives (Notify, Next disc).

Each item also carries a **context**: what the steps it passed learned
about it, each fact tagged with its source. A step uses what its items
carry and nothing else (an encoder writes the chosen release as tags,
with the cover); see *Data moves only along wires*. A step that *needs*
a fact is only offered once a step before it finds that fact.

### Establish, look up, choose, rip

The CD recipe in that order:

1. **Audio CD establishes the kind.** It reads the table of contents, so
   it knows this is an audio CD with 17 tracks of known length. The
   tracks exist as items from here on, before any of them is read.
2. **Each source is its own lookup step.** *MusicBrainz* and *CD-Text*
   sit side by side on branches from Audio CD and run at the same time.
   Each says what it found: *2 releases*, *17 titles*. Discogs and GnuDB
   go beside them.
3. **Choose joins them.** Each lookup offers candidates on the disc it
   passes on; CD-Text offers one, the disc's own text. Choose settles
   which of the candidates that reached it the disc is. Usually it
   decides by itself; when it's unsure, or only the disc's own CD-Text
   was found, it asks.
4. **Then the lookups that need the answer.** *Cover art* and *Lyrics*
   are asked with the chosen release, so they come after Choose. Cover
   art's one setting is the largest size to fetch (1200 px, 500 px or
   the original; 1200 px), since the picture it saves is the one every
   encoder after it embeds.
5. **Then Rip,** then the encoders and folders. Encoding starts on each
   track as soon as it's read, and every format runs in parallel.

Lookups take seconds, so ripping after Choose costs nothing in the usual
case. When Choose does ask, the drive waits for your answer.

The same order works for the other kinds of disc:

| Recipe | Kind step | Lookups | Then |
|---|---|---|---|
| Films and TV | Video disc (videos from MakeMKV) | TMDb | Choose (which film or show, and which video is which episode), Rip only those, Smaller video, Library |
| Games | Game disc (system and serial) | none before the rip | Rip, Check, Redump (by hash), Choose, CHD, Library, Copy protection, M3U |
| Data discs | Data disc (label, filesystem, size) | none | Rip, Check, CHD, Library |

A game's exact release is only known from the hash of its dump, so its
lookup comes after its rip. The order is the recipe's, not the app's.

### Folders, and holding

- **Folder** takes the files arriving from its left and puts them at its
  path, named by its template. Right after an encoder, that's the
  destination. After another folder, it's a copy from that folder to
  this one. So "review, then library" is two folder tiles.
- **Hold** is a step: it keeps the files that reach it until you let
  them through from its sidebar. Only that disc waits there; the drive and
  every other branch carry on, and the next disc can start. A held disc
  stays in the list under its recipe until you decide.

### Encoders

Each encoder has the settings someone who cares would look for, grouped
in its sidebar (the format's own, then *Tags and art*, then *Advanced*).
A setting that only applies in one mode shows only in that mode. The
tile's second line sums them up: *V0, joint stereo*; *level 8*.

An encoder uses whatever reached it: it writes every tag the track
carries, and embeds the cover when one came with it and the container
can hold one (FLAC, MP3, AAC, ALAC). None of that is a setting, and
nor is doing the job properly: every FLAC is decoded and compared with
the rip, and a mismatch fails that track.

| Encoder | Its own settings |
|---|---|
| FLAC | Compression 0–8 (8) |
| MP3 | Bitrate type VBR / CBR / ABR (VBR); Quality V0–V9 (V0) in VBR, Bitrate 128–320 (320) in CBR and ABR; Stereo joint / full (joint); Tag version 2.3 / 2.4 (2.3); Extra tags for very old players (off) |
| Opus | Bitrate 64–256 (160); Bitrate type VBR / constrained / CBR (VBR); Effort 0–10 (10); Tuned for music / speech / live calls (music) |
| AAC | Bitrate 128–320 (256) |
| Vorbis | Quality −1–10 (6) |
| ALAC, WAV | none |
| WavPack | Compression fast / normal / high / very high (high) |

Every encoder also has, under *Advanced*: extra ffmpeg arguments.

### One disc after another

**Next disc** is a step, and the flow shows it: the line out of it leads
back into the Drive tile. It goes on its own line from Rip, so the moment
the last track is read it opens the tray and the recipe starts again,
its Drive tile saying *Insert a disc* until the next one goes in. A
recipe without it does one disc. Stop ends the loop.

The disc before carries on encoding, filing, or holding for you. The
canvas shows the newest disc; earlier discs still going are listed under
their recipe, and clicking one shows its canvas. A disc leaves the list
when it's done.

### Notify

**Notify** takes anything and ends the recipe: once everything before
it is done, it says how the disc went (*17 tracks filed, all
verified*).

- **When:** *After every disc*; *Only if it needed you or something
  isn't right*; or *Never*. The middle one counts a question or a retry
  that waited on you during the run, and anything still wrong at the
  end: a track not verified, one skipped, or nothing filed. A Hold
  step waiting for you doesn't count; you put it there. A film's titles
  aren't checked tracks, so a film that went smoothly says nothing.
- **On this computer:** a desktop notification (the notification portal
  inside the flatpak).
- **Phone alerts (ntfy topic):** also publish to that topic on an ntfy
  server (`https://ntfy.sh` unless you set your own under *Advanced*),
  at high priority when something is still wrong at the end. The topic
  and server are set once for the app.

### Data moves only along wires

A step knows only its inputs and its outputs. Nothing is shared across a
run for steps to read.

- **An item carries what's known about it.** A disc carries what the
  steps before learned: the drive and its offset, the table of contents,
  the offers of the lookups it passed through, and after Choose the
  chosen identity alone (the other offers go no further). A track or a
  file carries its disc's knowledge and its own: its reads and verdict,
  then its tags. Rip takes the offset from the disc, where Drive put it;
  encoders, Folder, Cue, Cover art and Lyrics read the chosen release
  from the item in front of them, and Cover art and Lyrics add what they
  find to it.
- **A lookup offers candidates; it never decides.** MusicBrainz, Discogs,
  GnuDB, TMDb and Redump offer what they found. CD-Text offers one
  candidate, *From the disc (CD-Text)*, with the disc's titles and its
  artist and album when it has them. Choose's form starts from the
  candidates that reached it, nothing else.
- **Where branches join, the item knows what each branch's copy
  learned.** The join merges the copies, in the order of the wires, each
  fact once. So Choose's input is exactly the lookups wired into it: a
  lookup on another branch that doesn't lead into it is never one of its
  options.
- **A step may read its input items, its parameters and the settings,
  and nothing else.** Not another step's findings, not what a later
  step did with its own. The engine keeps a merge of what steps sent on,
  for the window; no step can read it. One exception: Notify asks the
  run whether any step waited on you (a question or a retry), since no
  item carries that.
- **So a record is a step's own.** It holds what came in, what the step
  did and gave, and its log. A lookup's Found shows what it offered;
  Choose's shows the options it received, which it chose, and why.
  Nothing says whether another step used a value.

### What follows what

Steps compose like functions: a step is offered at a + only if it takes
what arrives there and everything it needs has been found by a step
before it. *Needs* lists facts that must all have been found; `a | b`
means either will do.

| Step | Takes → gives | Needs | Finds |
|---|---|---|---|
| Drive | nothing → disc | | |
| Audio CD | disc → audio disc | | toc |
| Data disc | disc → data disc | | label |
| Game disc | disc → game disc | | serial, label, system |
| Video disc | disc → video disc | | titles |
| MusicBrainz, Discogs, GnuDB | audio disc → audio disc | | candidates |
| CD-Text | audio disc → audio disc | | candidates |
| Choose (audio) | audio disc → audio disc | candidates | release |
| Cover art | audio disc → audio disc | release | cover |
| Lyrics | audio disc → audio disc | release | lyrics |
| Rip (audio) | audio disc → tracks | | read |
| FLAC, MP3, Opus, AAC, ALAC, Vorbis, WAV, WavPack | tracks → audio files | | |
| Cue sheet | audio files → text files | filed | |
| Lyrics file | audio files → text files | filed; lyrics | |
| ReplayGain | audio files → audio files | | |
| TMDb | video disc → video disc | | candidates |
| Choose (video) | video disc → video disc | candidates | release |
| Rip (video) | video disc → titles | | read |
| Smaller video, Original video | videos → video files | | |
| Rip (data disc) | data disc → dump | | read |
| Rip (game) | game disc → dump | | read |
| Check | dump → dump | | hashes, name |
| Redump | dump → dump | system | candidates |
| Choose (game) | dump → dump | candidates | release |
| CHD, Plain image | dump → image file | | |
| Folder | any file → the same kind | | filed |
| Copy protection | image file → image file | filed; system; name \| release | |
| M3U | image file → image file | filed; name \| release | |
| Hold | any file → the same kind | | |
| Notify | anything → nothing | | |
| Next disc | anything → nothing | read | |

What that rules in and out:

- **Lookups on the disc come before Rip.** After Rip there are tracks,
  not a disc, so MusicBrainz or Cover art can't follow.
- **A lookup goes straight after what it looks up.** MusicBrainz,
  CD-Text, Discogs and GnuDB go on branches from Audio CD; TMDb right
  after Video disc; Redump right after Check; Cover art and Lyrics on
  branches from Choose. Chained after another lookup, the disc would no
  longer be its input. So there's no + after a lookup; a new one goes on
  a branch from the + after what it looks up.
- **Nothing identifies the disc once Choose has.** MusicBrainz, CD-Text,
  Discogs, GnuDB, TMDb, Redump and Choose itself aren't offered after a
  Choose; Cover art and Lyrics, which use the release, are.
- **No step twice on a path.** A step already on the way to a + isn't
  offered again. Folder and Hold may repeat (Review, Hold, Library).
- **Hold holds files.** It follows something that made files (an
  encoder, CHD, Smaller video…), never a disc: Choose already stops for you
  when it isn't sure.
- **Redump, its Choose and Copy protection are a game's.** Only Game disc finds
  the system they look in. Check, CHD, Keep and M3U take any dump.
- **Copy protection and M3U follow a Folder,** because they write beside the
  filed image under Redump's name for it.
- **Cue sheet and Lyrics file follow a Folder** too, for the same
  reason: they write beside the filed tracks, from their real names and
  format, so they have no settings. Each format's library that wants a
  cue sheet has its own Cue after it. A Lyrics file also needs Lyrics
  on the way, and writes an `.lrc` only for tracks with synced lyrics;
  the lyrics themselves travel on the tracks, and every encoder writes
  them as a tag.
- **Nothing follows Notify or Next disc.** They give nothing.
- **Branches join** into their common kind. Mixed kinds of file join into
  *any file*, so a Folder may follow. Anything else mixed, including a
  branch that ends in Next disc, joins into *anything*, after which only
  Notify and Next disc fit.
- **Deleting takes the step, then only what no longer fits without it.**
  Cover art out from between Choose and Rip leaves Rip joined to Choose;
  Rip out takes the encoders, which have no tracks to take. The sidebar
  lists what else would go before you confirm.
- **Old recipes still load.** They store step names, not kinds. A saved
  `file` kind reads as *any file*; `library` reads as Folder; a step
  marked to hold becomes a Hold step in front of it; a lookup saved
  chained after another is moved onto its own branch. A Cue sheet saved
  where it took the tracks from Rip moves after the Library of the
  format it named (FLAC if none), and the Folder that filed its sheet
  goes; Lyrics' old *.lrc files too*, when on, becomes a Lyrics file
  after each Library. Values saved for settings that no longer exist
  are ignored.

## The built-in recipes

From `crates/steps/src/recipes.rs`, seeded into `recipes.json` the first
time the window opens.

| Recipe | Steps |
|---|---|
| CDs | Drive, Audio CD, MusicBrainz and CD-Text, Choose, Cover art, Rip, then FLAC, Review, Hold, Library; MP3, Library; and Next disc |
| Archive everything | the most complex recipe (above) |
| Data discs | Drive, Data disc, Rip, Check, then CHD, Library; and Next disc |
| Games | Drive, Game disc, Rip, Check, Redump, Choose, then CHD, Library, Copy protection, M3U; and Next disc |
| Films and TV | Drive, Video disc, TMDb, Choose, Rip, then Smaller video, Library (named for Jellyfin); and Next disc |

They name the simulated drives (*Blu-ray drive*, *DVD drive*). On real
hardware they say *Not connected* until you pick one of your drives from
the list in the Drive tile's sidebar. Importing the first app's settings
is planned for the swap (milestone 5).

## The code

The shapes that matter, simplified from `crates/engine`:

```rust
/// What travels between steps.
enum ItemKind { Nothing, Disc, AudioDisc, DataDisc, GameDisc, VideoDisc,
                Track, Title, Dump, AudioFile, ImageFile, VideoFile, TextFile,
                AnyFile, Any }

/// What a step says about itself: read by the type check, the + picker,
/// the sidebar and the tile.
struct StepInfo {
    kind, name, from, group,          // "rip", "Rip", "cyanrip, AccurateRip", Read
    input: ItemKind, output: ItemKind,
    params: Vec<ParamSpec>,           // its settings; scope recipe, drive or app
    retries: Vec<RetryOption>,
    needs, gives, unless, right_after, // what follows what
    repeatable, passes_through, loops_back,
}

#[async_trait]
trait Step: Send + Sync {
    fn info(&self) -> StepInfo;
    async fn run(&self, inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()>;
    async fn find(&self, key, drive, measure) -> Option<(Value, String)>; // e.g. the offset
}

/// A recipe, as saved, edited and run.
enum Node {
    Step { id, step, label, params },  // every param is a Setting<Value>
    Chain { nodes },                   // a strip
    Branches { branches },             // a split; what follows joins them
    Group { name, body },              // a named box, acting as one step
}
```

- **One type check** over the tree. The + picker is the same check asked
  "what fits here?".
- **One runner.** Each step is a Tokio task fed by a channel; branches
  get every item; a join waits for its branches.
- **Every setting is a `Setting<T>`:** `Unset`, `Default` (not stored,
  so better defaults reach you), `Chosen { at }` or `Detected { source,
  at }`. An unset required setting makes its tile ask. A setting is
  scoped to the recipe, to a drive (the offset), or to the app (keys).
  Only chosen or detected values are written.

### Lanes

| Lane | Runs at once | Why |
|---|---|---|
| Drive | one per drive | a drive reads one thing, in order |
| Encode | one per CPU core | every (track, format) pair is independent |
| Network | one per service, rate-limited | MusicBrainz allows one request a second |
| File | one per destination | parallel copies to one share are slower |

Two drives run side by side.

## Building it

**Beside, not inside.** The new app was built as a Cargo workspace next
to the first app (`src-tauri` and a Svelte frontend), which stayed frozen
until the swap deleted it; it's in git history. Leaf modules (a tool's
arguments and output parsing, a catalogue client) were lifted across
when a step needed them, and tidied as they came.

```
Cargo.toml          the workspace
crates/engine       the domain: Setting<T>, the recipe tree, the Step
                    trait, the type check, the runner and its lanes, the
                    run snapshot and commands; no UI and no tools
crates/steps        one module per step, real and simulated, the tool
                    wrappers they use, and the built-in recipes
crates/layout       where tiles, groups and wires go on the canvas
crates/cli          `drt`: runs a recipe in the terminal
app/                the window, in egui
packaging/flatpak   the flatpak manifest; scripts/build-flatpak.sh builds it
```

To watch the engine run a simulated CD: `cargo run -p drt -- --simulate`
(`--recipe archive` for the most complex one, `--bad 9` for a track that
needs a retry, `--no-offset` for setup). The window does the same with
`cargo run -p drt-app -- --simulate`.

**The engine talks to the UI in two directions only:** a snapshot of a
run (each step's state, status, detail, progress and items; the open
question) that it publishes as it changes, and commands into it (answer,
retry an item, let a hold through, stop). The window, the CLI and the
tests all use the same two.

**The window is tested headless.** `app/tests/ui.rs` drives the real
window on simulated steps through its accessibility tree (egui_kittest)
and saves screenshots of each state to `app/shots/`. `app/tests/demo.rs`
renders the docs' screenshots and the demo video (ignored; run by hand).

**The accuracy fixes are tests of the new steps, not patches to the
first app.** Tags come from ffmpeg, never cyanrip (whose 0.9.3 corrupts
titles with a colon); when a track doesn't verify, Rip reads it again
carefully and compares the reads; Stop keeps what was finished. The
installed first app keeps the colon bug until the swap.

**The first app's configuration is read, never written.** A drive's
offset falls back to the one it recorded. Importing its folders and
settings is part of the swap.

### Where it stands, 19 November 2026

| | Built | Run on real hardware |
|---|---|---|
| 1. Engine | yes | yes, under the CD recipe |
| 2. Toolkit | egui | yes |
| 3. A real CD | yes, plus Lyrics, Discogs, GnuDB, cue sheet and log, ReplayGain, Notify (desktop and ntfy) | the CD recipe, end to end, on a 32-track disc (32 of 32 verified); the extras not yet |
| 4. The window | yes: its own frame, the drive watcher, typed *what follows what*, motion | daily use; the frame not yet checked on GNOME by eye |
| 5. The swap | no; the flatpak still ships the first app | |
| 6. Data discs | yes | no data disc yet |
| 7. Games | yes | no game disc yet |
| 8. Films and TV | yes | no DVD yet; MakeMKV is the flatpak `com.makemkv.MakeMKV` |

### Milestones

From milestone 3 on, each one is done when it works from a flatpak
build, not just `cargo run`.

| | What | Done when |
|---|---|---|
| 1 | The engine, with simulated steps | `drt --simulate` plays every mockup state (ripping, asking, a retry, a hold, setup, the most complex recipe); the tree, type check, lanes, questions and holds have tests |
| 2 | The toolkit spike | the most complex recipe and the *Which release?* drawer drawn natively from a simulated snapshot at 1280 × 720 |
| 3 | A real CD, without a window | `drt` rips a real CD with the CD recipe: Drive, Audio CD, MusicBrainz, CD-Text, Choose, Cover art, Rip with verdicts and retries, FLAC and MP3, Review and Library with its hold; the three accuracy fixes pass |
| 4 | The window | the list of recipes, the canvas, live tiles, the sidebar (questions, inside a step, setup), the + picker, editing, new recipes |
| 5 | Parity and the swap | the flatpak ships the new app; `src-tauri` and the web frontend deleted (done); the first app's settings (`config.json`) are not imported |
| 6 | Data discs | an image, checked against Redump, CHD, filed ("Other kinds of disc", 1) |
| 7 | Games | PlayStation then the other Redump systems (2) |
| 8 | Video | DVD and Blu-ray with MakeMKV and TMDb (3) |

Two drives at once isn't a milestone: lanes are per drive from the
start.

The toolkit is **egui**; see `docs/adr/0001-native-window-in-egui.md`
for what it was judged on.

## Other kinds of disc

Each a recipe made of steps like the CD one. The first app is read for
what it learned, not copied: what it got right is lifted into a step
and tidied; what it got wrong is listed so it isn't repeated.

### 1. Data discs: an image

The simplest: no lookup and no question, because a data disc is what its
label says.

`Drive, Data disc, Rip, Check, then CHD, Library; and Next disc`

| Step | Does | Settings |
|---|---|---|
| **Data disc** (kind) | reads the volume label, filesystem, size, and whether it's a CD, DVD or Blu-ray | none |
| **Rip** (read; data disc) | dumps the disc with redumper, re-reading each bad sector up to *Retries* times, then always a refine pass over any still bad; if some remain, it needs you: *Re-read bad parts*, *Keep it anyway*, *Skip this disc* | retries (20), read speed |
| **Check** | hashes the image and looks it up in Redump's catalogues, so a disc Redump knows gets its proper name and a *verified* | catalogues to check |
| **CHD** (make) | compresses losslessly into one file, and always verifies it gives back the image | compression, hunk size |
| **Plain image** (make) | the uncompressed image, exactly as read, with redumper's log | none |
| **Folder** | files it by template: `{name}` (Redump's, when known), `{label}`, `{system}`, `{date}`, `{size}`… | folder, names |

Lessons: CHD runs MAME's CHD library in-process (`libchdman-rs`), for
real progress and a working Stop; a CD's cue becomes a CD CHD, an ISO a
DVD CHD. The first app's one *Image* mode that tried to also be the
games mode is not repeated: games are their own recipe.

Done when a real data CD and DVD are imaged, checked, made into CHD,
verified and filed, from a flatpak build.

### 2. Games

`Drive, Game disc, Rip, Check, Redump, Choose, then CHD, Library,
Copy protection, M3U; and Next disc`. PlayStation first, then the other systems
Redump catalogues that a PC drive reads.

- **Game disc** (kind) reads the system and serial from the disc in
  milliseconds (`SYSTEM.CNF` for PlayStation).
- **Rip** is redumper's full dump, handling bad sectors as a data disc's Rip does:
  retried, then refined, with *Re-read bad parts* as a retry when some are
  left.
- **Check** hashes the dump; **Redump** gives the candidates by hash,
  else by serial, and **Choose** only asks when the hash matches nothing.
  A dump that matches nothing is held as suspect until you say.
- **Copy protection** (LibCrypt) puts Redump's `.sbi` beside a protected PlayStation disc;
  **M3U** keeps a multi-disc game's playlist beside its discs.

Lessons: a serial names a game, not a pressing (`SLUS-00152` is six
Tomb Raider revisions), so identity waits for the hash. Redump's
datfiles are one format for every system, cached and used offline.

Done when a PlayStation disc is verified against Redump and filed as
CHD, and a two-disc game gets its playlist.

### 3. DVD and Blu-ray video

`Drive, Video disc, TMDb, Choose, Rip, then Smaller video, Library; and
Next disc`.

- **Video disc** (kind) lists the videos (the disc's *titles*) with
  MakeMKV: length, chapters, audio and subtitle tracks, and which videos
  duplicate others. Videos shorter than a minute are skipped (a setting).
- **TMDb** searches by the disc's label and gives films or a series'
  episodes. Its API key is set once for the app.
- **Choose** matches videos to the film, or to episodes, one per video,
  with the best guess picked.
- **Rip** reads only the chosen videos, with MakeMKV.
- **Smaller video**: H.265 or AV1, quality, preset, 10-bit, which audio and
  subtitle languages to keep. Whether to deinterlace isn't a setting:
  it reads the stream's field order, else runs ffmpeg's `idet` on a
  stretch of the video, and logs what it decided and why. **Original video** keeps the video as it is and
  only drops tracks.
- **Library** names for Jellyfin: `{title} ({year}) [tmdbid-…]`, and
  `Season 03/… - S03E01 - …` for episodes.

Lessons: MakeMKV's beta key expires every two months; it's refreshed
only when MakeMKV says so. MakeMKV isn't bundled, so the Video disc tile
says when it's missing, and where to get it.

Done when a film and a season disc are ripped, matched, and filed for
Jellyfin.

## Not designed yet

- **Which recipe takes a disc.** Today a recipe reads from its drive
  when you press Start. The rule planned: a disc that goes in is taken by
  the recipe whose drive and kind step match it; if two match, the one
  used last takes it and the other's line offers *Use for this disc*; a
  disc no recipe takes appears at the top of the list with *Make a
  recipe*. Not built.
- **A record of finished discs**, and the log, outside a run.
- **The few things that belong to the app** rather than to any step (the
  working folder, MakeMKV's key).
- **Uploading to Proton Drive**, a step after a Library. Wanted, but
  waiting on Proton: as of November 2026 there is no OAuth and no way to
  grant an app access to only one folder. Third parties must use the
  official Proton Drive SDK, which Proton says isn't ready for
  third-party production use yet (CLI tools were planned for mid 2026;
  standalone use comes at general availability). rclone's Proton backend
  logs in with the full account (password and 2FA) through a
  reverse-engineered API. Neither meets the bar: the app must only ever
  be able to add files under one chosen folder, never read or delete
  anything else. Revisit when the SDK reaches general availability and
  check whether it offers scoped access; until then, a Library can file
  into a folder Proton's own desktop app syncs.

Each should be designed from the pipeline out, and anything that can
live on a tile should.
