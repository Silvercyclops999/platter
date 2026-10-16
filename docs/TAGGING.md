# Tagging rules

Anything Plex already reads uses Plex's own tag, written the way Plex expects.
Archive details that Plex has no field for go in `ARCHIVE_*` tags. Plex ignores
those, so Plexamp and other Plex apps keep working as before.

In MP3 files the tag editor writes the release type as `TXXX:MusicBrainz
Album Type`, as Mp3tag and Picard do, or in a `TXXX:RELEASETYPE` frame when
the file already has one. Check against current Plex docs if Plex ever stops
reading it: the forum thread that first documented the field is gone.

## Tags Plex reads

| Field | FLAC / Vorbis | MP3 (as Mp3tag writes it) | Notes |
| --- | --- | --- | --- |
| Album artist | ALBUMARTIST | TPE2 | Decides the artist page. Required on every track, or Plex treats the album as Various Artists |
| Track artist | ARTIST | TPE1 | Featured artists go here only, never in the title |
| Album | ALBUM | TALB | Same value on every track of an album or edition |
| Title | TITLE | TIT2 | |
| Track / disc | TRACKNUMBER, DISCNUMBER | TRCK, TPOS | |
| Year | DATE | TDRC | Album-level in Plex, so keep it identical across the album |
| Genre | GENRE | TCON | Album-level in Plex |
| Release type | RELEASETYPE | MusicBrainz Album Type (Mp3tag RELEASETYPE) | `album`, `ep`, `single`, `album;live`, `album;compilation`, `album;soundtrack`, `album;remix`, `album;demo`. Secondary types need both parts |
| MusicBrainz IDs | MUSICBRAINZ_ALBUMID, MUSICBRAINZ_TRACKID, … | MusicBrainz TXXX fields | Used for matching, Lidarr, library import |
| Cover art | Embedded or `cover.jpg` in album folder | Same | Artist photo: `artist-poster.jpg` in artist folder |
| Lyrics | Sidecar `.lrc` (synced) or `.txt`, same filename as track | Same | |
| Explicit | ITUNESADVISORY (1 explicit, 2 clean); EXPLICIT=1 also accepted | TXXX:ITUNESADVISORY (Mp3tag ITUNESADVISORY); M4A uses the `rtng` atom | Plex ignores it. Platter reads it from the file itself and shows an E badge |

## Archive tags (ignored by Plex)

| Tag | Values | Purpose |
| --- | --- | --- |
| ARCHIVE_VERSION | released, alternate, demo, session, leak, snippet, remix, live, build | Version badge, Other versions grouping |
| ARCHIVE_ERA | free text | Era cards and filters |
| ARCHIVE_QUALITY | lossless, cdq, lq, snippet | Quality badge |
| ARCHIVE_DATE | YYYY, YYYY-MM or YYYY-MM-DD (recorded or leaked) | Era dates, build order, sorting |
| ARCHIVE_EDITION | e.g. Standard, Deluxe, Build v2 | Edition picker label |
| ARCHIVE_GROUP | shared key across editions/builds of one album | Collapses them into one album card |
| ARCHIVE_SOURCE | free text | Source note |
| ARCHIVE_FLAGS | fanedit, aiedit | Hidden by default when set |

ARCHIVE_DATE is read in exactly those three forms, such as `2019`, `2019-05`
or `2019-05-31`. Anything else, like `Summer 2019` or `05/2019`, is ignored,
and the era falls back to the album years.

Songs count as versions of each other when the album artist (or the song's
own artist) matches and the titles match once version notes like `(Demo)`,
`- Snippet`, `[v2]` or `(Remastered 2011)` and featured artists are set
aside. Albums sharing an ARCHIVE_GROUP join their songs by title too. `Part
II` and `Part 2` are different titles, and generic titles like `Intro` only
match on the same album.

## Release status

Platter works out whether each file is released, unreleased, leaked or an
alternate version (a demo, an alternate take, a reference track, a snippet or
an original file) from the tags your library already uses. You don't have to
retag anything. It reads, in this order, and the first one with a value it
knows decides:

1. Platter's own `ARCHIVE_VERSION` (above). `leak` is leaked, `alternate` is
   alternate, `unreleased`, `demo`, `snippet`, `session` and `build` are
   unreleased, and any other version, like `released`, `remix` or `live`, is
   released.
2. A plain `STATUS` tag, with values like `Released`, `Unreleased` or
   `Leaked`.
3. The release status MusicBrainz Picard writes: `RELEASESTATUS` in FLAC and
   Ogg files, `MusicBrainz Album Status` in MP3 and M4A files. `official`,
   `promotion` and `withdrawn` are released, `bootleg` and `cancelled` are
   unreleased, and `pseudo-release` says nothing either way.
4. Picard's release type (`RELEASETYPE`, or `MusicBrainz Album Type`): a
   `demo` part makes the file an alternate version.

Values are read in any case, with `_` and `-` read as spaces. Besides the
values above, Platter knows `official`, `retail` and `promo` (released),
`unofficial`, `bootleg`, `scrapped` and `vault` (unreleased), `leak`
(leaked), and `alt`, `alternate`, `demo`, `OG`, `OG file`, `reference`,
`snippet` and `session` (alternate). Anything in brackets is set aside when
the whole value isn't known, so `Official release (web rip)` counts as
released. A value Platter doesn't know stays unknown. It never counts as
released.

A plain `VERSION` tag is free text, like `OG file` or `Alt version`. Platter
shows it as the file's provenance note and never reads it as
`ARCHIVE_VERSION`. Without one, `ARCHIVE_SOURCE` is the note.

Platter reads these from FLAC and Ogg Vorbis comments, MP3 `TXXX` frames and
M4A freeform (`----`) items, the same way it reads the `ARCHIVE_*` tags.

### A tag of your own

If your library keeps release status in another tag, open Settings, Library,
Archive and set Release status tag to Custom tag with its name, like
`PROVENANCE`. Platter then reads that tag after `ARCHIVE_VERSION`, instead
of `STATUS` and Picard's tags, and reads your files again in the background.
Under Release status values you can say what your own values mean, like
`Vault copy` for unreleased. Your values are checked before the built-in
ones, so they can also change what a built-in value means. Changing the
values doesn't read any files again.

### Albums

An album is rated by its files. When more than half of the files on disc 1
that have a known status are released, the album is official, even when a
later disc holds unreleased, leaked or alternate songs. Otherwise the most
common other status rates it. An album whose files hold both released songs
and other material is marked mixed, so an official album with a disc of
extras is official and mixed, and an unreleased build with a few released
songs is unreleased and mixed. Each song still keeps its own status, so a
leaked song on a bonus disc is never shared as a released one.

`ARCHIVE_VERSION` on an album's files works as it always has: the version
most of its files carry rates the album.

Platter only writes `STATUS`, `VERSION`, your custom tag or Picard's tags
when you change them in the tag editor, and then in the album's own scheme
(below).

## How material shows up

| Material | RELEASETYPE | Plexamp | This app |
| --- | --- | --- | --- |
| Unreleased builds, leaks, sessions | `album;demo` | Demos | Unreleased, with era + edition picker |
| Mixtapes | `album;mixtape/street` | Albums | Mixtapes |
| Alternate editions of a released album | same as released | Separate albums | One card + edition picker |

## Files and folders
- `Album Artist/Album/01 Title.ext`, with `Disc 1`, `Disc 2` folders for multi-disc.
- The app never renames or moves your files unless you ask it to.

## Plex library settings the app checks on first connect
- Embedded tags enabled; local metadata preferred.
- Personal Media agent available for albums not on MusicBrainz, so unreleased
  albums aren't overwritten by a wrong match.

The check only reads. Settings, Library, Tag editor shows it for every music
library with the steps in Plex for anything that's off, and Platter says so
once after it first connects. Plex Music libraries have no separate embedded
tags setting: they read the tags whenever local metadata is preferred.

## Tag editor and tag check

### The music folder
Platter edits tags in place, on the server's share. Settings, Library, Music
folder is where this computer sees the server's music: a mapped drive or a
network path (`\\server\music`) on Windows, a mounted share
(`/Volumes/Music`) on a Mac. Plex's path for each file is matched to it by the
longest end of the path found in that folder, and 5 random songs must be there
with the size Plex has ("Connected, 5 of 5 files found"). Until it's set, tag
details are view only.

### What the editor writes
"Edit tags..." on an album (or a song) edits the album fields (album artist,
album, year, genre, release type in two parts, edition, group, era, quality,
explicit) and each song's track and disc numbers with their totals, title,
artist, release status, version note, date, source, flags, explicit and ISRC.
When the songs differ, a field shows "(several)" and stays as each file has it
unless you change it.

| Field | FLAC / Ogg | MP3 | M4A |
| --- | --- | --- | --- |
| Album artist, album, title, artist | ALBUMARTIST, ALBUM, TITLE, ARTIST | TPE2, TALB, TIT2, TPE1 | aART, ©alb, ©nam, ©ART |
| Year, genre | DATE, GENRE | TDRC (TYER in v2.3), TCON | ©day, ©gen |
| Track and disc | TRACKNUMBER, TRACKTOTAL, DISCNUMBER, DISCTOTAL (or `n/total` when the file keeps them that way, or TOTALTRACKS and TOTALDISCS when it uses those) | TRCK, TPOS as `n/total` | trkn, disk |
| Release type | RELEASETYPE | TXXX:MusicBrainz Album Type | ----:com.apple.iTunes:MusicBrainz Album Type |
| Explicit | ITUNESADVISORY (or EXPLICIT when the file uses it) | TXXX:ITUNESADVISORY | rtng |
| ISRC | ISRC | TSRC | ----:com.apple.iTunes:ISRC |
| Archive tags, STATUS, VERSION, custom tag | as named | TXXX:name | ----:com.apple.iTunes:name |
| Picard's release status | RELEASESTATUS | TXXX:MusicBrainz Album Status | ----:com.apple.iTunes:MusicBrainz Album Status |

Only these fields are written, and only the ones you changed. Every other tag,
picture and frame stays as it was, in its place, and nothing new is added
besides what you set. An MP3 keeps its ID3 version: v2.3 stays v2.3, anything
else is written as v2.4. The audio itself is never touched.

The release status goes where the album already keeps it: whichever of
`ARCHIVE_VERSION`, the custom tag from Settings, a plain `STATUS` or Picard's
`RELEASESTATUS` most of its files carry. An album with none uses the custom
tag when Settings names one, else `ARCHIVE_VERSION`. The version note is a
plain `VERSION` tag.

### Saving safely
Before writing, every file is read again. When any file changed since the
editor opened it, or another program changed a file in its folder in the last
minute, nothing is saved. Each file is copied to a temp file in its own
folder, the tags are written to the copy, the copy is read back (its tags must
be what was asked and its length the original's), and only then does it
replace the original. Any failure leaves the original as it was. Platter
never renames or moves a file.

After saving, Plex rescans just those folders (Settings, Rescan in Plex after
saving) and Platter's own tag cache takes the new values. The last 20 edits
keep what they replaced: "Undo" puts it back on every file that hasn't changed
since. A save of more than 50 files asks first (Settings).

"Mark as explicit" and "Mark as clean" on an album, or on an era's page, set
explicit on every song after showing the changes.

### The tag check
Settings, Library, "Check tags" checks the library; "Check tags..." on an
artist or album checks just that. Each finding says what's wrong and why it
matters, with "Fix" (the editor, at that field) when the music folder is set,
and "This is fine" to stop showing it. It learns each album's own style from
most of its files (how ARTIST joins names, where featured artists go, how
files are named) and shows it.

It flags:
- a missing album artist (an error)
- a year, album name or album artist that differs within an album
- a release type missing its first part, like `demo` for `album;demo`
- an ARCHIVE_GROUP with only one album
- one folder that Plex shows as two albums
- ARCHIVE_* values and tags Platter doesn't know, and dates in other forms
- missing disc numbers or totals on albums with more than one disc
- a release status missing where the album's other songs have one, and gaps
  in the provenance note
- ARTIST joining names differently from the rest of the album, and featured
  artists placed differently
- repeated or missing track numbers on a disc, and a title twice
- file names in another pattern than the album's, and numbers or titles that
  disagree with the file name (a `/` in a title and ` - ` in its file name,
  characters a file name can't hold, and case alone don't count)
- ripper and player tags (ReplayGain, iTunNORM, encoder) that most of the
  album doesn't use
- songs under a minute, MP3s under 192 kbps, and released songs without an
  ISRC, as information only

### One-click fixes
Each fix lists every change it would make and what it leaves alone before
anything is written, then saves through the editor, so it can be undone.
- Fill disc tags: `1/1` on songs of single-disc folders. A folder whose file
  names (`1-01 - Title`), tags or folder names show more than one disc is left
  for the editor.
- Fill release status: a suggestion for each song without one, from the other
  songs on its disc. Albums often mix statuses on purpose, so none is ticked
  until you tick it.
- Look up ISRCs: for released songs without one. Pick the one official release
  the folder is from (MusicBrainz first, then Deezer's free public catalogue);
  songs are matched by title, artist and a length within 1.5 seconds. Songs
  whose length differs (a re-edit) or without a confident match are kept as
  they are, and listed. Only the ISRC is written: credits never change.
