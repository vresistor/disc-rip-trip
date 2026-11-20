# Covers from other editions, and every scan beside the tracks

20 November 2026.

## Why

Cover art embeds the front of the exact release Choose picked. Often that
release's only scan is small: one pressing of an album may have a 491 px
front while another pressing of the same album, in the same MusicBrainz
release group, has a 2813 px front and thirty more scans of its back,
booklet, tray and disc. The front is the same picture; only the scan
differs. And some people want those scans themselves, kept beside the
tracks.

## A bigger front from another edition

Cover art still fetches the chosen release's front at the size asked for
(`front-1200`, `front-500` or the original). The Cover Art Archive doesn't
enlarge a small scan, so the picture that comes back already says how big
the original is: no extra request in the common case. When its longer side
is under the size asked for, or the release has no front, and the new
setting *Use a bigger cover from another edition* is on (the default), it
looks at the other releases in the release group:

1. One MusicBrainz browse, `release?release-group=…&inc=labels`, through the
   same one-a-second throttle and busy retries as every MusicBrainz call. It
   lists each edition's label, catalogue number, country, date and whether
   the archive has a front for it, so editions without one cost nothing.
   At most 300 releases are read (three pages), and at most 12 editions
   with a front are measured, those with the most scans first.
2. Each candidate's original front is measured from its first 256 KB (a
   ranged request: a PNG or JPEG gives its size in its header), so a 10 MB
   scan isn't downloaded to learn it's 2813 px.
3. The edition with the longest side wins (ties: more scans, then the
   release id), if it's longer than the chosen release's own. Its front is
   fetched at the size asked for, so the archive does the scaling as now.

With *Original* no size is asked for, so it only looks elsewhere when the
release has no front at all. The browse, each release's list of images and
each measured size are kept in the cache for a week.

The card says *From another edition* (it has room for no more). The
sidebar says which, naming what tells the editions apart rather than what
they share: *From another edition: Polydor, Europe 1995*. The chosen one is
the UK pressing of the same Polydor catalogue number, so the country is what
differs; a different catalogue number is named when there is one. The
record adds the original's size: *…, 2813 px*. The edition used goes on the
disc as two facts, its id and those words, and the sidebar's archive link
opens that edition.

## Picking the cover yourself

Some scans are poor whatever their size, so Cover art has a second setting,
*Let me pick the cover*, off by default. When it's on, Cover art measures
every front as above (the release's own and up to 12 other editions'),
fetches each at 250 px, and asks *Which cover?* with the fronts as pictures
side by side, each with where it's from and its original size under it.
The one it would have taken by itself is picked to start with; *Use this
cover* fetches the picked one at the size asked for. A Choice in a question
can now carry a picture, and a question whose answers have pictures is
drawn as a grid of them rather than as buttons.

Not built: Discogs' images (another service, its own key and limits), and
picking a different cover after the fact from the sidebar to be remembered
for the next rip of the album, which needs somewhere to keep per-album
choices that the steps don't have yet.

## Artwork files

A new step after a Folder, offered and checked like *Lyrics file*: it needs
files saved and an album chosen. On the first track that arrives it takes
the album's folder (that track's folder) and saves every image the archive
holds for the chosen release into `Artwork` inside it, at original size,
named for what they show: `Front.png`, `Back 01.jpg`, `Booklet 01.png`,
`Disc 01.png`, `Tray.jpg`, `Matrix.jpg`, with each image's own extension.
A name used more than once is numbered in the archive's order; the release's
own front is always plain `Front`.

If the chosen release has no images at all, it uses the edition Cover art
used (the fact on the disc), or failing that the same search, and says so on
its card. Taken literally this means a release with only a small front gets
just that front: the step saves the chosen release's scans when it has any.

Downloads go one at a time through the archive's network lane, into the
run's working folder, with each image a row and the step's progress bar
counting bytes. The drive is long free by then: the step follows a Folder.
Saving reuses the Folder's rules: the same bytes already there count as
saved, a different file under the same name waits, and once the rest are
saved the Folder's one question is asked (*Replace them*, *Keep both*,
*Skip*). Its pictures are a new kind of item, *pictures*, so nothing that
expects a disc image or a text file is offered after it.
