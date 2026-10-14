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
| MusicBrainz IDs | MUSICBRAINZ_ALBUMID, MUSICBRAINZ_TRACKID, … | MusicBrainz TXXX fields | Used for matching, Lidarr, Spotify import |
| Cover art | Embedded or `cover.jpg` in album folder | Same | Artist photo: `artist-poster.jpg` in artist folder |
| Lyrics | Sidecar `.lrc` (synced) or `.txt`, same filename as track | Same | |

## Archive tags (ignored by Plex)

| Tag | Values | Purpose |
| --- | --- | --- |
| ARCHIVE_VERSION | released, alternate, demo, session, leak, snippet, remix, live, build | Version badge, Other versions grouping |
| ARCHIVE_ERA | free text | Era cards and filters |
| ARCHIVE_QUALITY | lossless, cdq, lq, snippet | Quality badge |
| ARCHIVE_DATE | YYYY-MM-DD (recorded or leaked) | Era timeline, sorting |
| ARCHIVE_EDITION | e.g. Standard, Deluxe, Build v2 | Edition picker label |
| ARCHIVE_GROUP | shared key across editions/builds of one album | Collapses them into one album card |
| ARCHIVE_SOURCE | free text | Source note |
| ARCHIVE_FLAGS | fanedit, aiedit | Hidden by default when set |

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
