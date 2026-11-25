# Security

## Reporting a vulnerability

Please report it privately, not in a public issue: on GitHub, open the
**Security** tab of this repository and choose **Report a vulnerability**.
Say what you found, how to reproduce it, and which version or commit.

You'll get a reply within a week. Once a fix is out, the report is
published with credit to you, unless you'd rather not be named.

## What's supported

The latest release, and `master`.

## What counts

Anything that lets a disc, a metadata service (MusicBrainz, Discogs, GnuDB,
TMDb, Redump, LRCLIB, AccurateRip) or a file in your library make Disc Rip
Trip write outside the folder you chose, run something, crash, or send your
keys, token, email address or ntfy topic anywhere but their own service.

The flatpak's permissions are broad on purpose, and why is written in
`packaging/flatpak/io.github.vresistor.disc-rip-trip.yml`. The bundled tools (cyanrip,
redumper, ffmpeg) are other projects: please report their bugs upstream too.
