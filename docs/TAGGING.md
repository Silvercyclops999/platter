# Tagging rules

Anything Plex already reads uses Plex's own tag, written the way Plex expects.
Archive details that Plex has no field for go in `ARCHIVE_*` tags. Plex ignores
those, so Plexamp and other Plex apps keep working as before.

Check the MP3 release-type field name against current Plex docs before
writing tag code. The forum thread that originally documented it is gone.

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

Platter never writes `STATUS`, `VERSION`, your custom tag or Picard's tags.
They change only when you edit them yourself.

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

## Tag editor and tag check
The tag editor edits a whole album at once, writes only the fields above, and
then asks Plex to rescan just that folder.

The tag check flags a missing album artist, a year or album name that differs
within an album, a release type missing its second part, and an ARCHIVE_GROUP
with only one member.
