# What each step did

Every step keeps a record of its own run: what came in, what it did and
gave, and its log. Only its own: a step knows its inputs and its outputs
(DESIGN-UX.md, *Data moves only along wires*), and its record says
nothing of what a later step did with what it gave. Its drawer shows that record live, and it is
kept with the run, so a finished step still shows what happened.

**Why:** a lookup's findings are fed into the next step and lost. Today
CD-Text's tile says *17 titles*, and nothing shows which titles, or why
the file names came out as they did.

## The record

One per step, per run. The engine fills it from what the step sends on;
steps add to it through `Cx`.

| Part | What it holds | Filled by |
|---|---|---|
| **In** | what arrived: kind, count, from which steps | the engine, as items pass |
| **Found** | facts the step added to the items it sent on (subject, key, value), each tagged with the step | the engine, as items leave the step |
| **Offered** | candidates the step offered on the item it sent on, with scores | the engine, as items leave the step |
| **Options** | for Choose: the candidates that reached it, as they came | `cx.options` |
| **Decided** | for Choose: which candidate, and why (*only one*, *best match 0.97*, *you picked*, *entered by hand*) | `cx.decided` |
| **Out** | what left: kind and count, to which steps; files made, with path and size | the engine; steps that write files say which |
| **Log** | the step's own lines, timestamped | `cx.log` |
| **Tool output** | the raw lines from the tool it runs (cyanrip, redumper, makemkvcon, ffmpeg); the last 2,000 kept in memory, all of them in a file in the run folder | `cx.raw` |
| **Times** | started, finished | the engine |

No step's record says whether another step used its values: CD-Text's
says what it offered, Choose's what it chose among and why.

## The drawer

The header stays as it is: pill, sentence, primary action. Under the
sentence, one line of flow:

> **In** 17 tracks from Rip · **Out** 17 FLAC files, 412 MB → Review

The right-hand side of the drawer gets tabs. A step only shows the tabs it
has something for:

- **Items**: the per-item list, as today (Rip, encoders, Folder, Hold).
- **Found**: what it worked out, presented by what it is.
  - *Facts*: a table, the disc's row first, then one row per track. Columns
    are the keys found (length, ISRC…).
  - *Candidates*: what a lookup offered, best first, with score and
    detail. CD-Text's is one: *From the disc (CD-Text)*.
  - *A decision* (Choose): the chosen candidate as a card and the reason, with
    the other options that reached it below; before it decides, the
    options alone.
  - *An image* (Cover art): the picture, its size and where it came from.
  - *Files* (encoders, Folder, CHD, Transcode): name, size and folder; a
    folder can be opened.
- **Log**: timestamped lines, newest at the bottom, following while it runs
  unless you have scrolled up. *Tool output* is a fold beneath, shown in a
  monospace font. It opens by itself when the step failed.

Which tab opens first: *Needs you* items if there are any; a failed step
opens on Log; a lookup or Choose opens on Found; any other step opens on
Items.

## Kept

When a run ends, its records are written to `runs/<recipe>/last.json` in the
app's data folder, and the tool output to a file beside it. One run per
recipe is kept, replaced by the next. After a restart, a recipe's tiles
show the last run's outcome, and their drawers show its records, until a
new run starts.

## Not in this

- Hovering a wire to see what passed along it.
- A history of more than the last run.
