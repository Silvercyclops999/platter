# IPC contract (UI ↔ Rust core)

The UI never talks to Plex, the network, files or audio directly. It calls the
Tauri commands and listens for the events listed here. Types are in `src-tauri/src/model.rs` (Rust, canonical) and
mirrored in `src/lib/types.ts`. Wire format is camelCase JSON.

The UI reaches the core only through `src/lib/backend.ts`. Inside the app it
uses the real Tauri bridge; in a plain browser it uses an in-memory mock with
fixture data, so screens can be built, screenshotted (Playwright) and tested
(Vitest) without a server.

Errors: commands return `Result<T, AppError>`; `AppError` serializes as
`{ kind: string, message: string }` where `kind` is one of `network`,
`auth`, `notFound`, `server`, `io`, `decode`, `invalid`, `internal`.

## Ids
Item ids are opaque strings. A Plex item's id carries its server:
`<tag>:<rating key>`, where the tag is eight hex digits of a hash of the
server's machine id (the same tag share links use, never the id itself), so
the same number on two servers is two items. Plex image paths carry it the
same way (`<tag>:/library/metadata/1/thumb/2`) and the `art` protocol takes
them as they are. Part keys don't; they always go with their track.

Ids from before this (a plain rating key) still work. Upgrading rewrites the
cache in place, in one transaction, once the connected server is known, and
remembers that server as the one plain ids belong to. Plain ids in a saved
queue, links, the sync file, follows, mix exclusions and the UI's own saved
state map to it. Downloaded files and cached art and songs keep their names.
`src/lib/ids.ts` has the UI's helpers.

## Artwork
A custom URI scheme, served by Rust from the disk cache (LRU, 2 GB default):
`art://localhost/<urlencoded plex image path>?w=<px>&h=<px>`
On Windows (WebView2) the URL form is `http://art.localhost/...`, so always use
`artUrl()` in `backend.ts` instead of building it by hand. It takes the
rendered size in CSS px and asks for that times the display's pixel ratio,
rounded up to a fixed set of sizes (`src/lib/artSize.ts`) so nearby sizes
share cache entries. On a cache miss Rust
fetches `/photo/:/transcode` at the requested size. The token never appears in
the URL.

## Auth and servers
| Command | Args | Returns |
| --- | --- | --- |
| `auth_status` | – | `AuthStatus` |
| `auth_start` | – | `PinInfo { code, authUrl, expiresAt }` (also opens the browser) |
| `auth_cancel` | – | – |
| `auth_sign_out` | – | – |
| `servers_list` | – | `ServerInfo[]` |
| `server_select` | `{ machineId }` | – |

`AuthStatus = { state: "signedOut" | "pending" | "signedIn", username?, avatar?, server?: ServerInfo, needsServerPick: bool, reauthRequired: bool }`

`avatar` is the Plex account's profile picture as an art path
(`/ext/https://plex.tv/users/<id>/avatar?c=<n>`), served by the `art`
protocol and cached like other art. The core reads it from plex.tv's user
call at sign-in and start. It only accepts plex.tv hosts and drops any token
from the URL, and the picture is fetched without one (plex.tv redirects to
the image on assets.plex.tv). For a Plex Home it is the signed-in user's
picture. Missing when the account has none.

`reauthRequired` means Plex rejected the saved sign-in (expired or revoked).
The app stays open so music on this computer keeps playing, and the health
banner asks the user to sign in again.
`ServerInfo = { machineId, name, owned, online, version?, connection?: "local"|"remote"|"relay" }`

Event `auth://changed` → `AuthStatus`.

### Home users
| Command | Args | Returns |
| --- | --- | --- |
| `home_users` | – | `HomeUsers { users: HomeUser[], remember: bool }` |
| `home_switch` | `{ uuid, pin?: string }` | – (the app restarts as that user) |
| `home_remember_set` | `{ on: bool }` | – |

`HomeUser = { uuid, name, avatar: string \| null, admin, restricted, protected, current }`.
`users` is empty when the account isn't a Plex Home (or has one user).
`protected` means switching needs that user's 4-digit PIN; it's typed by the
user, sent once to plex.tv in a form body and never stored. A wrong PIN is
`{ kind: "invalid", message: "That PIN isn't right." }` and changes nothing.

Each Home user has their own data folder (`users/<key>/` under the app data
folder, `key` a hash of their uuid; the user who signed in keeps the top
folder) and their own keychain entries (`u.<key>.` names). A switch gets the
user's token from plex.tv, stores it in their entries, records the choice in
`home-user.json` and restarts the app, which saves the queue first. Nothing
on a Plex server changes. `remember` (Settings, this device) off starts as
the user who signed in. Signing out as another Home user goes back to the
user who signed in.

Event `home://changed` → `{ restarting: bool }` just before the restart.

### More than one library
| Command | Args | Returns |
| --- | --- | --- |
| `library_sources` | – | `LibrarySources { servers: SourceServer[] }` |
| `library_servers` | – | `LibraryServers { servers: { tag, name }[], view }` |

`SourceServer = { machineId, tag, name, owned, current, online, connection?, libraries: { id, title, included, main }[] }`.
`library_sources` lists every server on the account, owned or shared, with
its music libraries, asking each server at once (a server that doesn't
answer is listed offline, with the libraries included from it). The
connected server comes first and its main library (`main`) is always in.
Which others are in is the `libraries` setting (`<machineId>/<sectionId>`
entries); changing it starts a sync, which reads the libraries added and
drops the rows of those left out.

Each included library syncs into the same cache with its own rows. An album
on two servers is found by its MusicBrainz release id (Plex `Guid`), or else
by artist, title, track count and every song's length within 2 seconds;
songs pair up within it, and songs elsewhere match on their MusicBrainz track
id. The main library's copy stands for the group, else the best file.

`library_servers` is for the sidebar switcher: the connected server and every
server with an included library, without asking them. `view` is the
`serverView` setting ("all", or a server's tag), back to "all" when that
server isn't included any more. With "all", lists show each album, song and
artist once and an artist page has its albums from every server; a server's
tag shows only its music. Changing it fires `library://changed`.

When a song with copies on several servers is queued, the copy played is the
one on the server reached the best way (local network, then remote access,
then the relay), then the better file (lossless, bit depth, sample rate,
bitrate). Plays, likes, play queues and Plex players get that copy's id, so
they go to the server that has the file. A playlist lives on one server: new
ones go to the server with their songs, and songs from another server can't
be added to it.

## Library
| Command | Args | Returns |
| --- | --- | --- |
| `sync_status` | – | `SyncStatus` |
| `sync_now` | `{ full?: bool }` | – |
| `home` | – | `HomeData { shortcuts: HomeItem[], recentlyPlayed: HomeItem[], jumpBackIn: Album[], playlists: Playlist[], recentlyAdded: Album[], topArtists: Artist[] }` |
| `home_shelf` | `{ shelf: "recentlyPlayed"|"jumpBackIn"|"playlists"|"recentlyAdded"|"topArtists", limit?: number }` | `HomeItem[]` (the whole shelf, for Show all; limit defaults to and caps at 200) |
| `artists` | `{ sort: "name"|"recent"|"plays", offset, limit }` | `Page<Artist>` |
| `artist_names` | – | `{ id, title }[]` (every artist, then learned names with `credit:` ids) |
| `albums` | `{ sort: "name"|"artist"|"year"|"recent"|"plays", offset, limit, section?: ReleaseSection }` | `Page<Album>` |
| `playlists` | – | `Playlist[]` |
| `library_items` | – | `LibraryItem[]` (every playlist, album and artist, unordered) |
| `artist_page` | `{ id }` | `ArtistPage { artist, popular: Track[], sections: { section, albums: Album[] }[], plays, about?, similar: Artist[], appearsOn: Album[], popularPending }` |
| `album_page` | `{ id }` | `AlbumPage { album, tracks: Track[], totalDurationMs, moreByArtist: Album[], artistThumb?, released?, studio? }` (`released` is YYYY-MM-DD and `studio` the label, both from Plex when it has them) |
| `recent_tracks` | `{ limit?: number }` | `Track[]` played on this device (the listen log), newest first, repeats kept; limit defaults to 50 and caps at 200 |
| `recents` | `{ limit?: number }` | `RecentsEntry[]` for the Recents page, newest first (see below) |
| `playlist_page` | `{ id }` | `{ playlist, tracks: Track[] }` |

`Page<T> = { items: T[], total }`.
`HomeItem` is `{ kind: "album", album }`, `{ kind: "artist", artist }` or
`{ kind: "playlist", playlist }`. Home rows hold 12 items. Recently played
counts a play started from an artist or a playlist for that artist or
playlist, not for each song's album; Plex history from other devices counts
for the album. `shortcuts` holds up to 8: the 4 albums, artists and playlists
played most in the last week (each play's weight halves every day, so
several plays today beat one, and plays from Liked Songs don't count), then
the most played album, artist and playlist in turn, then the rest of those
and other recent ones, with no repeats. The UI puts Liked Songs first and
shows 7 of them when anything is liked. `playlists` puts the ones played on this computer first,
newest first, then the rest by title. `topArtists` is artists by play count,
played ones only.

`recents` reads the listen log for the last 90 days (300 entries at most).
`RecentsEntry` is `{ kind, item?, at, tracks: Track[] }`. Plays are grouped
into listening sessions: back-to-back plays from the same album, artist,
playlist, Liked Songs (`kind: "liked"`) or from no single context
(`kind: "songs"`), with less than 30 minutes between one song's end and the
next one's start. `kind: "context"` carries the album, artist or playlist as
a `HomeItem` in `item`. Skips aren't plays and are left out. Likes in the
same window show as `kind: "likedAdded"`, grouped the same way by their
rating time. `at` is the newest play or like in unix ms, and `tracks` is
newest first with repeats kept.
`artist_names` feeds the artist links in track lines: the UI loads it once,
keeps it, and loads it again after `library://changed` (and shortly after
`artistinfo://changed`, which can teach new names). It ends with names that
contain a separator, like "Tyler, The Maker", learned from ListenBrainz
artist credits, so a split keeps them whole; their ids are `credit:` ids.

`popular` holds the artist's own songs and songs where the artist is
credited as a whole name in the track artist line, at most 10, one copy per
song. With `popularSource` "global", songs on the artist's global top list
(see Artist info) come first in list order, matched by title folded for
comparison (case, "feat." credits, edition notes like "2011 Remaster" or
"Bonus Track" and punctuation ignored). Version qualifiers such as
"(Interlude)", "(Live)", "- Demo", "(Radio Edit)" or "(Radio Sessions)" count,
so a variant only matches a list entry with the same qualifier. When several
copies match one entry, one shows: the copy on the release the list names,
then released over unreleased, a studio album over other release types, the
earliest year, lossless then higher bitrate, most played. The rest follow by
your own plays, then most recent play, with variants of listed songs after
the others; a track's `ratingCount` only breaks ties there. Hidden songs are
left out, and so are songs by hidden artists on other artists' pages (a
hidden artist's own page still lists its songs). When there is no
list, the whole list is in plays order. `popularPending` is true while
`popularSource` is "global", a source for the list is on and none has been
fetched for the artist yet; the page then shows placeholder rows until
`artistinfo://changed` brings the list, `artistinfo://popular` says the fetch
finished without one, or 1.5 s pass. `appearsOn` lists albums by others whose
tracks credit the artist, newest first.

Every name in a track line that has no library artist gets a credited
artist page with the id `credit:<name in lower case>` (while the
`linkFeatured` setting is on). `artist_page` with such an id builds the page
from the tracks that credit the name: `artist` is made up (no thumb or art,
title as written in the first credit), `popular` is ranked like above,
`appearsOn` lists the albums of those tracks, `plays` sums their plays, and
`sections` and `similar` are empty. A name nothing credits (or only as part
of a longer name) is `NotFound`. Playing such a page with `play_context`
kind `"artist"` plays Popular, then the other songs that credit the name.
`search` lists credited artists among `artists` when their name matches.
On the artist page, `plays` is the sum of play counts over the artist's own
tracks, `about` is the Plex artist biography (null when empty), and `similar`
holds the Plex similar artists that are in this library, at most 12, in
Plex's order. Artist lists never carry the biography. Similar artists come
from the artist's detail, not from sync: `artist_page` answers with what's
cached and, when that is missing or a week old, fetches it in the background
and then emits `artist://changed` with `{ id }` so the page can ask again.

`LibraryItem = { kind: "playlist"|"album"|"artist", id, title, sortTitle, creator?, creatorId?, thumb?, smart, format?, addedAt?, playedAt? }`
feeds the library sidebar, which sorts and filters it in the UI. `creator` is
the album artist and `creatorId` its id (albums only). `addedAt` is Plex's addedAt for albums and the
earliest of an artist's albums for artists; playlists have none. `playedAt` is
the latest play in Plex history or the local listen log (and Plex's
lastViewedAt) for albums, an artist's newest album play, and the latest local
play started from a playlist. Times are unix seconds.
`SyncStatus = { phase: "idle"|"artists"|"albums"|"tracks"|"incremental", done, total, lastSyncAt? }`
Event `sync://progress` → `SyncStatus`; event `library://changed` (no payload)
after any sync writes, so open pages refetch.

Between syncs the cache follows the server's notification socket. Items Plex
finishes adding or changing, and items it deletes (its `timeline` entries for
artists, albums and tracks in the music library), are gathered for 2 seconds
and written straight into the cache: changed items are read again in one
`/library/metadata/{k1},{k2},…` call, gone ones are dropped, and
`library://changed` fires once for the batch. New albums by followed artists
reach What's New and its notification the same way. A batch of more than 200
items (a big scan) runs an incremental sync instead, as do the socket
connecting, missed events and a scan ending. While the socket is down an
incremental sync runs every 15 minutes.

## Playlist editing
| Command | Args | Returns |
| --- | --- | --- |
| `playlist_create` | `{ title?, trackIds? }` | `Playlist` |
| `playlist_add` | `{ id, trackIds, after?, top? }` | `Playlist` |
| `playlist_duplicates` | `{ id, trackIds }` | `string[]` (the ones already in it) |
| `playlist_remove` | `{ id, itemIds }` | `{ itemId, trackId, index }[]` (what was removed and where) |
| `playlist_move` | `{ id, itemIds, after? }` | – |
| `playlist_update` | `{ id, title?, summary? }` | `Playlist` |
| `playlist_set_cover` | `{ id, bytes: number[], contentType }` | – |
| `playlist_delete` | `{ id }` | – |

`Playlist` also carries `summary` (the description) and `updatedAt` (unix
seconds, the last change). `playlist_page` returns `itemIds` next to
`tracks`, one per row in the same order. Edits name rows by item id because
a song can be in a playlist more than once.

Plex playlists are the store. Every edit changes the local cache at once,
emits `library://changed`, and is queued for Plex. The queue replays in
order as soon as the server is reachable and every 15 seconds after that,
so edits made offline reach Plex later. If Plex refuses an edit, the rest of
that playlist's queued edits are dropped, a `toast` says so, and the
playlist is read back from Plex. A playlist deleted in Plex disappears here
on the next read.

`playlist_create` without `title` names the playlist from the
`playlistNewName` setting, where `#N` becomes the next number no playlist
uses ("My playlist #3"). Plex makes playlists from a list of songs, so a new
empty playlist exists only on this computer until its first song is added;
then it's created in Plex with everything it has by then. Its id changes to
the Plex id, and the old id keeps working in every command and in
`playlist_page`.

`playlist_add` appends at the end, at the top with `top: true`, or right
after the row `after`. Titles are 1 to 100 characters and descriptions up to
300 (`""` clears it); longer ones are an `invalid` error. Covers are JPEG,
PNG or WebP under 10 MB. Smart playlists can't be edited (`invalid`).

`playlist_duplicates` lets the UI ask before adding a song that's already
there (setting `playlistAskDuplicates`).

## Edit in Plex
| Command | Args | Returns |
| --- | --- | --- |
| `plex_edit_access` | – | `{ show, servers: string[], connected, lockFields, matchUnreleased }` |
| `plex_edit_item` | `{ id }` | `EditInfo` |
| `plex_edit_images` | `{ id, which: "poster" \| "art" }` | `{ key, preview, provider, selected }[]` |
| `plex_edit_select_image` | `{ id, kind, which, key }` | `EditChange` |
| `plex_edit_upload_image` | `{ id, kind, which, bytes: number[], contentType }` | `EditChange` |
| `plex_edit_image_link` | `{ id, kind, which, url }` | `EditChange` |
| `plex_edit_matches` | `{ id, title?, year? }` | `{ guid, name, artist, year, score }[]` |
| `plex_edit_match` | `{ id, kind, guid, name, year? }` | `EditChange` |
| `plex_edit_unmatch` | `{ id, kind }` | `EditChange` |
| `plex_edit_details` | `{ id, fields }` | `EditChange` |
| `plex_edit_merge` | `{ target, others: string[], photoFrom? }` | `EditChange` |
| `plex_edit_split` | `{ id }` | `EditChange` |
| `plex_edit_refresh` | `{ id, kind }` | `EditChange` |
| `plex_edit_undo` | `{ changeId }` | `EditChange` (now `undone`) |
| `plex_edit_history` | `{ id }` | `EditChange[]`, newest first |

These change Plex's own data for an album or artist (`kind` is `"album"` or
`"artist"`), never the files. Only servers the account owns can be edited:
`servers` lists their tags and `connected` covers ids without a tag. A
server that answers 401 or 403 to an edit is an `auth` error ("Only the
server's owner and admins can edit this.") and drops out of `servers` for
the rest of the run. With `plexEditShow` off every write is `invalid`.

`EditInfo` has `id`, `kind`, `fields`, `locked` (Plex field names),
`guid`, `thumb`, `art`, `artistTitle`, `sectionId`, `matched` (not a
`local://` guid) and `unreleased` (albums, by the same rule as sharing).
`fields` is `{ title, sortTitle, released, studio, summary, genres, styles,
moods }`; `released` is `YYYY` or `YYYY-MM-DD`. `plex_edit_details` sends
only the fields that differ and locks them when `plexEditLock` is on. Artists
have no `released` or `studio`.

`which: "art"` is an artist's background. Uploads are JPEG, PNG or WebP under
10 MB; links must be http or https. After an image change the item's cached
artwork is dropped, so the new one shows at once. Every change is read back
into the library cache (`library://changed`); merge and split also start a
sync, since albums move between artists.

`EditChange` is `{ id, itemId, kind, action, label, at, undoable, undone }`,
`action` one of `image`, `match`, `unmatch`, `details`, `merge`, `split`,
`refresh`. Undo puts back the previous image (by selecting it again, or
uploading the copy kept when it was replaced), the previous match, the
previous field values and locks, or splits a merge. Split and refresh can't
be undone. The last 100 changes are kept on this computer.

With `plexEditMatchUnreleased` off, `plex_edit_match` on an unreleased album
is `invalid`. The UI hides Fix match there; Unmatch always works.

## Likes
| Command | Args | Returns |
| --- | --- | --- |
| `likes_set` | `{ trackIds, liked }` | `string[]` (the ids whose state changed) |
| `liked_ids` | – | `string[]`, newest like first |
| `liked_page` | – | `{ tracks: Track[], totalDurationMs }`, newest like first |
| `playlists_with_track` | `{ trackId }` | `string[]` playlist ids, most recently changed first |

A liked song is a Plex user rating, so other Plex apps see the same likes.
Liking sets the rating to 10 (five stars), even over a lower rating;
unliking clears it. Which ratings count as liked is the `likedMeans` setting
(five stars, or four and up). Liking a song that already counts as liked
leaves its rating alone.

Every change is applied to the local cache at once and queued for Plex, one
write per song, replayed in order as soon as the server is reachable and
every 15 seconds after that. If Plex refuses a write, the song's rating goes
back to what Plex has, a `toast` says so, and `likes://changed` fires again.
Each sync also reads every rated track from Plex, so likes made in other
Plex apps arrive with the next sync (the server's library events start one).
A song with a write still queued keeps its local rating.

`Track` carries `ratedAt` (unix seconds, when it was last rated: the "date
added" of a liked song) and `genres` (the track's own Plex genre and mood
tags, the Liked Songs filter chips). Both are left out when empty.

Event `likes://changed` → `{ ids: string[] }` after a like or unlike here, a
rollback, or a change read from Plex. Liked Songs plays with
`play_context` kind `"liked"` and id `"liked"`; with `trackIds` it plays those
in that order (the page's sort or filter).

## Artist info
| Command | Args | Returns |
| --- | --- | --- |
| `artist_info` | `{ id }` | `ArtistInfo` |
| `album_credits` | `{ id }` (album) | `AlbumCredits` |
| `album_release_status` | `{ id }` (album) | `AlbumReleaseStatus` |
| `artist_info_keys` | – | `{ lastfm, fanart, audiodb, listenbrainz: { saved, rejected } }` |
| `artist_info_key_set` | `{ source: "lastfm"|"fanart"|"audiodb"|"listenbrainz", key: string|null }` | `{ saved, rejected }` |
| `artist_info_cache_clear` | – | number of artists that had cached info |
| `open_info_link` | `{ url, artistId? }` | – (opens a source, licence or profile page in the default browser) |

`ArtistInfo = { id, bio?, bioSource?: "plex"|"wikipedia"|"lastfm"|"audiodb", origin?, yearsActive?, members: { name, artistId?, years? }[], related: { name, artistId?, relation }[], tags: string[], listeners?, scrobbles?, listenersFrom?: "lastfm"|"listenbrainz", photo?, banner?, logo?, credits: { source, text, url?, licence?, licenceUrl? }[], gallery: string[], links: { kind, label, url }[], bioLinks: { text, artistId }[] }`

Optional outside sources fill gaps that Plex leaves: Wikipedia and Wikidata
(no key, on by default), Last.fm, Fanart.tv and TheAudioDB (each needs the
user's own key and is off until one is saved). `artist_info` answers from a
local cache at once and fetches missing or month-old entries in the
background, then emits `artistinfo://changed` with `{ id }`. Only the
artist's name or MusicBrainz id is sent. Sources that are off, failing or
whose key was refused are left out, along with their credits.

MusicBrainz (no key, on by default) gives members with their years, related
artists ("Member of", "Subgroup", "Collaboration", "Formerly" and so on) and
the years active when Wikidata has none. ListenBrainz listener numbers (on by
default) fill `listeners` and `scrobbles` when Last.fm has none, with
`listenersFrom` saying which one they came from.

`AlbumCredits = { albumId, state: "off"|"pending"|"none"|"found", released?, editionDate?, label?, credits: CreditRow[], tracks: { trackId, credits: CreditRow[] }[], url? }`
with `CreditRow = { name, artistId?, roles: ("producer"|"writer"|"composer"|"lyricist"|"featured"|"mixer"|"engineer"|"mastering")[] }`.

`album_credits` answers from the same cache at once. A missing or month-old
entry is `pending` and is looked up in the background, then
`albumcredits://changed` fires with `{ id }`, found or not. The lookup uses
the MusicBrainz release id Plex has for the album (one request), else a sure
release group match by artist and title and its official release with the
same number of songs. `released` is the release group's first date,
`editionDate` this release's own date when it differs. Album `credits` lists
everyone on the release and its songs except featured artists, who stay with
their songs in `tracks` (matched to library tracks by disc and position, else
by title). Unreleased albums are never looked up (`none`). MusicBrainz is
asked at most once a second with a user agent that names the app; a failed
lookup waits ten minutes before it's tried again.

`AlbumReleaseStatus = { id, artistId, status: "taggedUnreleased"|"confirmedUnreleased"|"probablyUnreleased"|"plexMissed"|"released", reasons: string[], evidence: Evidence[], mark: "unreleased"|"released"|null, score }`
with `Evidence = { kind: "tag"|"mark"|"markOverridden"|"unmatched"|"untyped"|"musicbrainz"|"notChecked"|"folder"|"duration", text, weight, decides }`.
`reasons` are the evidence as plain sentences. Tags decide first: a demo
release type or an unreleased `ARCHIVE_VERSION` is `taggedUnreleased`, and
any other version tag is `released`. The user's mark comes next
(`confirmedUnreleased` or `released`); a mark a tag disagrees with is listed
as `markOverridden`. Then any other release type is `released`. An album
Plex couldn't match with no tags is `probablyUnreleased` (while
`countUnmatchedUnreleased` is on), unless the MusicBrainz check found it,
which makes it `plexMissed`, a released album. Folder names on the songs'
paths (`archiveFolderKeywords`) and most songs being under a minute add
weight to the list but never change the status. `score` is the weights
summed. The status of every album is also stored in `album_status` after
each sync, tag scan, mark and change to the settings that feed it.

The MusicBrainz check (`checkUnmatchedMusicbrainz`, needs MusicBrainz on)
runs in the background after each library sync. For each album Plex couldn't
match that has no release type and no version tag on the album or its files,
it searches release groups by artist and title (edition notes like
"(Deluxe)" left out) and takes a sure match only: score 100, the same folded
title and the same artists, and an official release in the group whose track
count is within 2 of the album's. Found answers are kept 30 days, misses 7
days, in `album_mb_check`. It shares the one-a-second MusicBrainz pacing and
pauses after each album so page lookups go first. A found album's release ids
give Discord its Cover Art Archive cover, and a cover uploaded while it
counted as unreleased is forgotten. `releasestatus://changed` fires with
`{ id }` when an album's answer changes.

`bio` is Plex's summary when it is longer than 300 characters, else the
first of Wikipedia, Last.fm and TheAudioDB with more text than Plex has, else
Plex's. `photo` is set only when Plex has no artist thumb, and `banner` only
when Plex has no background art. `photo`, `banner` and `logo` are art paths
(`/ext/<image url>`) served by the `art` protocol and cached like Plex art.
`members` link to library artists by name. `credits` lists one line per
source whose data is shown, with its link and licence.

`gallery` holds up to 12 art paths for the About dialog: Plex's photo and
background first, then Fanart.tv's photos and backgrounds, then TheAudioDB's
photo and fan art. `links` are the official website and profiles (Instagram,
X, Facebook, YouTube, Bandcamp, SoundCloud) that the Wikidata item lists,
https only, then the Wikipedia article and the Last.fm page. They come from
answers already fetched; nothing extra is requested. `bioLinks` are library
artists the bio names (the text exactly as written, first mention each,
never the artist itself), for linking them in place.

The same background fetch gets the artist's global top list for Popular
while `popularSource` is "global": ListenBrainz top recordings by the
artist's MusicBrainz id (no key; an optional user token, sent as
`Authorization: Token`, lets it answer artists it hasn't cached), and when
ListenBrainz has nothing, Last.fm top tracks if Last.fm is on with a key.
Copies of a song are merged and the top 100, with the release names each came
out on, are kept for 7 days, "none found" too. Offline, busy or a bad answer
stores nothing and is tried again on a later page open. The list is fetched
before the other sources. A new list emits `artistinfo://changed`, and the
artist page then asks `artist_page` again; every finished list fetch, found or
not, also emits `artistinfo://popular` with `{ id }`.

At startup and after each library sync the core also fetches missing or stale
lists in the background for the artists on Home's shortcuts and Recently
played and the most played artists (60 at most), one artist every 20 seconds,
so most artist pages open with the list ready. Only the list is fetched this
way; the other sources wait for the page.

`artist_info` also takes `credit:` ids. Their name comes from the tracks that
credit it, and their MusicBrainz id from a MusicBrainz artist search, taking
only a result with score 100 whose name or alias equals the credited name;
Plex is never asked about them. The other sources then work as for any
artist, cached under the `credit:` id.

Keys are kept in the OS keychain, never in settings. Saving a key checks it
with one lookup; `rejected` means the service refused it, which turns that
source off until another key is saved or the app restarts. `open_info_link`
only opens https links on the sources', Creative Commons' and the profile
sites. With `artistId` it also opens a link listed in that artist's `links`
(their official website).

## Archive
| Command | Args | Returns |
| --- | --- | --- |
| `release_mark_set` | `{ kind: "album"|"track", id, mark: "unreleased"|"released"|null }` | – |
| `track_versions` | `{ id }` (song) | `VersionItem[]` |
| `era_page` | `{ artistId, name }` | `EraPage` |
| `album_section_set` | `{ albumId, artistId, name: string|null }` | – |
| `artist_sections` | `{ artistId }` | the artist's own section names, A to Z |

Nothing here writes to Plex or to files. Marks and sections are app state in
the sync store (`releaseMarks`, keyed `album:<id>` or `track:<id>`, and
`albumSections`, keyed by album id), so they sync with the library group.
A song's own mark beats its album's. After a mark, statuses are worked out
again for that artist and `releasestatus://changed` (`{ id }`, the album) and
`artist://changed` fire. Marks or sections from another device emit
`archive://changed`.

Songs are grouped with their other versions in `versions` after each sync,
tag scan and mark: the same album artist or song artist and the same title
once version notes and credits are folded away, a shared `ARCHIVE_GROUP` with
the same title, or the same recording id or Plex track id. Generic titles
("Intro") and one-letter titles only join on the same album. Each song gets
its own status: `released`, `unreleased`, `probablyUnreleased`, or
`probablyAlternate` for a song on a probably unreleased album whose title a
released song has. That is still unreleased.

Tracks on album, artist (Popular), playlist and search pages carry
`otherVersions?` (other versions, leaving out fan and AI edits unless shown
and copies on another server) and `songStatus?`.

`VersionItem = { track, status, version?, quality?, date?, era?, edition?, year?, current }`,
released versions first, then the newest by `ARCHIVE_DATE` or album year.

`search` shows one row per song. A song with other versions shows as its
released version when it has one, whatever ranked first, and the rest go in
`versions` under that song's id. The top result follows the same rule.

`artist_page` sections follow the release status: unreleased albums go to
`unreleased` (with `demos` split out when an artist has both demos and other
unreleased material), and albums moved to the user's own sections come last
with `custom` set to the name. `eras: Era[]` lists the artist's eras, oldest
first:
`Era = { name, start?, end?, trackCount, albumCount, cover?, coverAlbumId?, albumIds, trackIds }`.
Dates come from `ARCHIVE_DATE`, else the album years.

`EraPage = { artist, era, groups: { album, date?, edition?, tracks }[] }`,
newest build first. `album_page` adds `archive?: { era?, date?, edition?, source? }`
for unreleased albums with archive tags.

## Sharing
| Command | Args | Returns |
| --- | --- | --- |
| `share_info` | `{ kind: "track"|"album"|"artist"|"playlist", id, atSeconds? }` | `{ appLink, public: "available"|"unreleased"|"off"|"none" }` |
| `share_public_link` | `{ kind, id }` | the public link (string) |
| `deep_link_take` | – | `DeepLinkOpen` or null |

`DeepLinkOpen = { status: "open"|"notInLibrary"|"invalid", kind?, id?, albumId?, atSeconds?, join? }` (`join`: a listening session invite to pass to `listen_join`)

`appLink` is a `platter://` link: `platter://<song|album|artist|playlist>/<id>`,
plus `?t=<seconds>` for a song when `atSeconds` is given, and `s=<tag>`, eight
hex digits of a hash of the server's id (not the id itself). The id in the
link is the server's own, and the tag is the one of the server that has the
item (see Ids), so a link from another server isn't mistaken for whatever has
the same number here. Links are read leniently: `track` for `song`, `t` as
seconds or `m:ss`, a missing `s` (the connected server). `DeepLinkOpen.id` is
the id as the app keeps it, with its tag.

`public` says whether `share_public_link` can work: `unreleased` for music
whose album release type is a demo, whose `ARCHIVE_VERSION` says so, or whose
album Plex couldn't match and MusicBrainz doesn't have (see `countUnmatchedUnreleased`
and `album_release_status`; the one rule lives
in `db::release::is_unreleased`), `off` when the
`sharePublicLinks` setting is off, `none` for artists and playlists.

`share_public_link` searches the iTunes Search API with the artist and title
(songs) or the artist and album title (albums), nothing else, at most one
search every 3 seconds. A result counts only when artist, title and album all
match after folding case, accents, punctuation, featured artists and edition
notes, and a song's length is within 3 seconds. The link is the store item's
song.link page (`https://song.link/i/<id>`, `https://album.link/i/<id>`).
Answers are cached for 30 days, misses for a day. Errors: `notFound`
("Couldn't find this song online" or album) with no sure match, `invalid`
("Not released", or turned off), `network` or `server` when the search can't
be reached (not cached).

Opening a `platter://` link (the OS starts or focuses Platter with it) is
resolved in the core, kept until the UI takes it with `deep_link_take`, and
announced with event `deeplink://open` (no payload). A song opens its album;
with `atSeconds`, the player seeks there once when that song starts playing.

## Share cards
| Command | Args | Returns |
| --- | --- | --- |
| `share_card_qr` | `{ kind, id, atSeconds? }` | `{ size, rows, text }` |
| `share_card_copy` | raw body: RGBA pixels; headers `x-card-width`, `x-card-height` | – |
| `share_card_save` | raw body: a PNG; header `x-card-name` (percent-encoded) | the saved file's name, or null when cancelled |

Share › Share card… (songs, albums, playlists) and Share lyrics in the lyrics
view open a dialog with a 9:16 story card (1080 x 1920) and a square card
(1080 x 1080). The UI lays the card out and draws it on a canvas
(`src/lib/shareCard.ts`): the cover on a background tinted from it, title,
artist and the Platter mark; for the song playing now a progress bar at the
current time; for released music the song.link line (`share_public_link`,
never for unreleased music, and only with `sharePublicLinks` and
`shareCardSongLink` on); and a QR code when `shareCardQr` is on. A lyric card
puts up to 4 picked lines where the big cover would be.

`share_card_qr` makes the QR code in the core from the item's `appLink`
(`share_info`), so it only ever holds the `platter://` link with the 8-hex
server tag, never an address, the server's id or a token. `text` is what it
holds. `rows` are strings of `0` and `1`, one per row.

`share_card_copy` puts the image on the clipboard; the body must be exactly
width x height x 4 bytes, at most 4096 on a side. `share_card_save` checks the
body is a PNG and writes it where the save dialog says. Nothing is uploaded.
Errors: `invalid` for a wrong size or a body that isn't a PNG, `internal`
("Couldn't copy the image.") when the clipboard refuses.

## Miniplayer
| Command | Args | Returns |
| --- | --- | --- |
| `miniplayer_state` | – | `{ open }` |
| `miniplayer_set` | `{ open }` | `{ open }` |

Event `miniplayer://changed` → `{ open }` when the window opens or closes
(from either window, or because the main window closed).

The miniplayer is a second window, label `miniplayer`, with no frame and its
own capability (`capabilities/miniplayer.json`: dragging and resizing only).
It loads the same page as the main window, which sees the label and shows
just the miniplayer, fed by the same `player://` events. It opens at 340 x 365
in the bottom-right corner of the main window's screen, or where it was last
when that place is still on a screen (`miniplayerBounds`, saved a moment
after it stops moving and when it closes). It is always on top while
`miniplayerOnTop` is on; changing that setting applies at once. A change to
any `miniplayer*` setting is sent to both windows as `settings://changed`.
When the window can't be made, `miniplayer_set` fails with `internal` ("The
miniplayer couldn't open.").

## Menu bar (macOS)
On a Mac the core builds the menu bar: Platter, File, Edit, Playback, View,
Window and Help. The table of items is in `src-tauri/src/platform/menubar.rs`.
Windows has no menu bar and never sends this event.

Event `menu://action` → `{ action }` when an item is picked. `action` is one
of `checkUpdates`, `settings`, `newPlaylist`, `newFolder`, `search`,
`filterLibrary`, `playPause`, `next`, `previous`, `volumeUp`, `volumeDown`,
`shuffle`, `repeat`, `miniplayer`, `fullWindow`, `lyrics`, `queue`,
`toggleLibrary`, `shortcuts`, `goOffline` and `signOut`. The UI runs the same
code as its own buttons and shortcuts (`src/shell/menuBar.ts`): Go Offline
flips the `goOffline` setting, and Sign Out asks first, like the app menu.
Anything but a playback item, Miniplayer or Go Offline brings the main window
forward first.

Items with keys use the app's own shortcuts (Cmd+, for Settings, Cmd+Left
and Cmd+Right for previous and next, and so on); New Playlist (Cmd+N) and New
Playlist Folder (Shift+Cmd+N) are the only keys the menu adds. Play/Pause
shows no key, since Space belongs to the page. When the page has already
handled a key press, the menu's copy of the same press within half a second
is skipped, so a key never acts twice.

Quit Platter and Platter on GitHub are done in the core: Quit saves the queue
first, like the tray's Quit, and the project page opens in the browser. So
are Help's Report an Issue, which opens the project's issues page, and Show
Logs in Finder, which opens the folder holding `platter.log` (the core's info
level log, started fresh past 5 MB with one older file kept). About,
Services, Hide, the Edit items, Close Window and the Window menu are the
system's own. Play/Pause, Shuffle and Repeat follow `player://state` (the
label reads Pause while playing; Shuffle and Repeat show a checkmark when on),
and Go Offline's checkmark follows the setting.

## Now-playing feed
| Command | Args | Returns |
| --- | --- | --- |
| `feed_status` | – | `FeedStatus` |

`FeedStatus` is `{ state: "off" }`, `{ state: "on", port, url }`,
`{ state: "portInUse", port }` or `{ state: "failed", port, message }`.
Event `feed://status` carries it whenever it changes. The feed is turned on
and its port chosen with `settings_set` (`nowPlayingFeed`,
`nowPlayingFeedPort`); the core opens, moves or closes the port to match.

The feed itself is plain HTTP on `127.0.0.1:<port>` for widgets, not part of
the IPC. It only binds the loopback address, answers GET and HEAD, and only
when the `Host` header is `127.0.0.1:<port>` or `localhost:<port>` (403
otherwise). No CORS header is sent.

- `GET /now-playing.json` → `{ playing, private, title, artist, artists,
  album, art, positionMs, durationMs, trackId }`. `art` is
  `http://127.0.0.1:<port>/art?t=<trackId>`, or with `?art=data` a
  `data:image/...;base64,` URL. Nothing playing gives nulls and empty lists;
  a private session gives `private: true` and nothing else.
- `GET /now-playing.html` → the overlay page. Query options: `size` (small,
  medium, large), `theme` (dark, light, clear), `art` and `progress` (1 or
  0). It reads the JSON once a second.
- `GET /art?t=<trackId>&s=<px>` → the playing song's cover from the art
  cache (300 px unless `s` asks for 64 to 1200). Any other track id, or a
  private session, is 404.

## Stream Deck
| Command | Args | Returns |
| --- | --- | --- |
| `deck_status` | – | `DeckStatus` |
| `deck_revoke` | – | `DeckStatus` |

`DeckStatus` is the port's state as in `FeedStatus` (`off`, `on` with
`port`, `portInUse`, `failed`) plus `code` (the six-digit pairing code, only
while it's on) and `paired`. Event `deck://status` carries it whenever it
changes, including after a pairing. `deck_revoke` forgets the paired Stream
Deck and makes a new code. It's turned on with `settings_set`
(`allowStreamDeck`).

Request album and Listen along from a Stream Deck bring the window up and
send `deck://action` to the UI, which shows the usual confirmation:
`{ kind: "requestAlbum", input: LidarrRequestInput }` opens the Lidarr
request dialog, and `{ kind: "listenAlong" }` starts a session (or shows the
open invite).

The control API itself is plain HTTP with JSON on `127.0.0.1:32541`, for the
Stream Deck plugin in `streamdeck/` and other local programs. It binds the
loopback address only, answers only loopback peers whose `Host` is
`127.0.0.1:32541` or `localhost:32541`, refuses any request with an `Origin`
header or a `Sec-Fetch-Site` other than `none` (403), and allows 20 requests
a second (429 past that). Bodies are at most 4 KB.

- `GET /v1/hello` → `{ app: "Platter", api: 1, paired }`; `paired` says
  whether the bearer token sent is the paired one.
- `POST /v1/pair` with `{ code }` → `{ token }`. One try every two seconds;
  a wrong code is 401 and five in a row make a new code. A success also
  makes a new code and replaces any earlier token. Only the token's SHA-256
  is kept, in the keychain.
- Everything else needs `Authorization: Bearer <token>` (401 without it).
- `GET /v1/state?cover=<coverKey>` → `{ api, playing, private, title, artist,
  album, trackId, liked, volume, muted, coverKey, cover, canRequest }`.
  `volume` is 0 to 100. `cover` is a 144 px `data:image/...;base64,` image,
  left out when `cover` in the query already names it. A private session
  leaves out the song and cover.
- `POST /v1/command` with `{ command }`: `playPause`, `next`, `previous`,
  `like` (toggles), `volumeSet` and `volumeStep` (with `value` in percent),
  `requestAlbum`, `listenAlong` → `{ ok: true }`, or 409 with `{ error }`
  saying why (nothing playing, Lidarr not set up, no album to request).

## Discord
| Command | Args | Returns |
| --- | --- | --- |
| `discord_status` | – | `DiscordStatus` |
| `discord_imgur_key_set` | `{ key: string \| null }` | `{ saved, rejected }` |

`DiscordStatus = { available, connected, imgurKey: { saved, rejected }, uploadFailed }`

Event `discord://status` → `DiscordStatus`, when Discord connects or goes
away, a cover upload fails, or the client ID changes.

`available` is false when the app was built without a Discord application id
(`PLATTER_DISCORD_APP_ID` at compile time); then nothing runs and Settings
shows a note instead of the Discord rows. Otherwise the core follows the
player's track events and talks to the local Discord app over its IPC socket
(the `discord-ipc-0` to `-9` named pipe on Windows, the unix socket of that
name in the temp folder on macOS). It looks for Discord every 15 s while
something is playing and Discord isn't open. Losing Discord never affects
playback.

The status is a Listening activity (Playing if the client refuses that):
details = song title, state = "by " and the artists (featured artists split
out, joined with commas), large image = cover (the app icon when there's
none), large text = album (with "(paused)" while paused), no small image,
`status_display_type` from `discordStatusLine` (0 app name, 1 state, 2
details; left out from then on if the client refuses it, before falling back
to Playing), start and end timestamps while playing,
and a Find this song button with the song's song.link page for released songs
(found the same way as `share_public_link`). Text is cut to 128 characters.
Updates are gathered for 300 ms (so a gapless change is one update) and sent
at most once every 15 s, always with the latest state. A new song waits up to
1.5 s for its cover. Nothing is shown during a private session, while
`discordStatus` is off, while paused when `discordClearOnPause` is on, or for
unreleased songs when `discordShowUnreleased` is off.

The cover is a public URL, never a Plex URL. Cover Art Archive first, by the
album's MusicBrainz release id from Plex (`coverartarchive.org/release/<id>/front-500`,
checked once and kept 30 days, a miss 7 days). For a released album without
one, a MusicBrainz release group search by album artist and title (sharing
the artist pages' one request a second), taking only a score 100 result whose
title and artists match once case, accents, punctuation, featured artists and
edition notes are folded, and only when one group matches; its cover is
`coverartarchive.org/release-group/<id>/front-500`, cached the same way. A
search that fails isn't cached. Unreleased albums skip both. Then, with a
saved client ID and `discordUploadArt` on, the 512 px cover from the art cache
uploaded to Imgur and its `i.imgur.com` link kept for good. With
`discordUploadCovers` on (and after Imgur when Imgur fails), the cover goes to
the keyless hosts `discordCoverHost` picks, in this order for `"auto"`:
Litterbox (`litterbox.catbox.moe/resources/internals/api.php`,
`reqtype=fileupload`, `time=72h`, `fileToUpload`; a `litter.catbox.moe` link
kept 72 hours), x0.at (POST `file` to `x0.at`; kept 3 to 100 days by size),
uguu.se (`uguu.se/upload`, `files[]`, a JSON answer; a `*.uguu.se` link kept
3 hours) and catbox.moe (`reqtype=fileupload`, `fileToUpload`; a
`files.catbox.moe` link kept for good). The first host whose answer is a
direct image link on that host wins. The upload is the art cache's 512 px
cover, sent as it is when it's a JPEG of 200 KB or less, else scaled to fit
512 px and made a JPEG at quality 85. `discord_art` keeps the link with its
host and the time the host deletes it; a link is used until 12 hours before
that (half its life for a host that keeps files less than a day), then the
cover is uploaded again. Links kept from earlier builds stay in use. Only
these hosts are ever sent; anything else falls back to the app icon. A fresh
upload is fetched once from its link before the status is sent, so the host
has it ready, and a host that answers that fetch with something other than
an image is skipped. When every host fails, the upload isn't retried until
the next start or a new client ID.

Discord answers SET_ACTIVITY with the external cover rewritten as
`mp:external/<signed>/https/<host>/<path>`, and its client keeps a broken
image if its proxy's first fetch failed. So after Discord takes a status with
a cover, `https://media.discordapp.net/<the part after mp:>` is fetched once
in the background (a 200 with an image type is good). If it fails it's tried
again after 5 s, 15 s and 40 s. An uploaded cover that fails the first fetch
is also uploaded right away to the next host in the order that hasn't failed
this play (a kept catbox.moe link starts again from the top), and the status
is sent with the new link without waiting the 15 s; that link is checked the
same way. When a later try of the first link works before a new link is
sent, the same status is sent once more (still at most one update per 15 s)
so the client loads the cover again. When no link works and no host is left,
the song shows the app icon for this play. However they come about, at most 5
SET_ACTIVITY calls go in any 20 s; a change held back by that goes out when
there's room, as the latest state. Nothing
is remembered, so the next play tries the cover again. Logs name only the
image host, never the signed path. The client ID lives in the keychain;
saving one checks it with one request that uploads nothing, and `rejected`
is set when Imgur refuses it then or on an upload.

## File tags
| Command | Args | Returns |
| --- | --- | --- |
| `tags_index` | – | `TagIndex { tracks, albums, groups }` |
| `tag_scan_status` | – | `TagScanStatus { enabled, running, done, total, failed, paused }` |

Events: `tags://progress` → `TagScanStatus` as the scan moves (at most a few
times a second), and `tags://changed` (no payload) after each batch of files,
when badges, editions and filters should be read again.

Plex doesn't expose `ARCHIVE_*` tags or the explicit flag (docs/TAGGING.md),
so the core reads them from each file's header through the track's part URL
(`/library/parts/{id}/{stamp}/file.ext`, token in a header). The first read
is `Range: bytes=0-65535`. When the tags lie further in (a big cover picture
before them, or an MP4 whose `moov` atom follows the audio) the parser names
the next bytes it needs and one more 64 KB read (or the block's own size, up
to 1 MB) follows, at most six per file. Formats: FLAC Vorbis comments (with or
without an ID3 tag in front), ID3v2.2 to 2.4 `TXXX`, `TSRC` and `UFID`, MP4
`rtng` and `----` items, Ogg Vorbis, Opus and FLAC comments. Kept:
`ARCHIVE_VERSION`, `_ERA`, `_QUALITY`, `_DATE`, `_EDITION`, `_GROUP`,
`_SOURCE`, `_FLAGS` and explicit (`ITUNESADVISORY` 1 or 4, `EXPLICIT=1`, M4A
`rtng` 1 or 4; 2 means clean). Also kept, to tell when two files hold the same
song: the ISRC (ID3 `TSRC`, Vorbis `ISRC`, MP4 `ISRC`) and the MusicBrainz
recording id (Vorbis `MUSICBRAINZ_TRACKID`, ID3 `UFID` owned by
`http://musicbrainz.org`, MP4 `MusicBrainz Track Id`). Songs are matched by
ISRC, then the recording id from file tags, else Plex's track id, then
artist and title. Each cached row carries the cache version it was read
with; when a new version keeps more, older rows are read again after every
unread file, and they keep working until then.

Reads go one at a time through the player's transfer gate at the lowest
priority, below pre-caching, and only while playback has nothing left to
download. Any playback transfer that wants the gate cuts the read, which is
sent again later. A server that ignores Range gets a whole-file read only for
files under 2 MB; bigger ones are skipped. Results are cached in SQLite by
part key, which carries the part's update stamp, so a changed file is read
again and its old row dropped. A file that can't be read (404, bad data) is
marked and left until it changes. Network or server trouble, or a relay-only
connection, pauses the scan (`paused`) and it tries again a minute later.

`tracks` maps track ids to `{ explicit?, version?, quality?, edit? }` (only
songs with something to show). `albums` maps album ids to `{ explicit?,
version?, group?, edition? }`: explicit when any song is, the most common
version, edition and group. `groups` maps a group key to its editions
`{ albumId, title, label, year }[]`, two or more, the face first. The face is
the edition labelled Standard, else a released one, else the earliest.

`ARCHIVE_VERSION` joins the unreleased check for sharing and Discord: any of
unreleased, leak, demo, snippet, session, build or alternate counts. With
`groupEditions` on, `albums`, `artist_page`, `home` and `home_shelf` leave out
editions behind their face, and `album_page` leaves its own editions out of
More by. Play contexts and album pages follow `explicitContent` and
`showFanEdits` (see Settings). A context that loses every song fails with
`invalid`.

## Now playing
| Command | Args | Returns |
| --- | --- | --- |
| `now_playing_details` | `{ trackId }` | `NowPlayingDetails { trackId, artist?, credits, lyrics? }` |

The extras the now-playing panel shows for a track, read from Plex when the
panel needs them (not cached in the library database).
`artist = { id, title, summary?, thumb?, art? }` is the album artist with its
Plex bio and photo. `credits = { name, roles: ("composer"|"writer"|"lyricist"|"producer")[] }[]`
lists people Plex credits beyond the track's artists; it is often empty.
`lyrics` (the `Lyrics` shape below, from Plex only) is there only when the
track has a lyrics stream with words in it. Only the track request can fail
the call: when the artist or lyrics request fails, that part is just left out.

| Command | Args | Returns |
| --- | --- | --- |
| `lyrics_get` | `{ trackId }` | `Lyrics \| null` |
| `lyrics_own_get` | `{ trackId }` | `OwnLyrics \| null` |
| `lyrics_own_save` | `{ trackId, text, prefer }` | – |
| `lyrics_own_delete` | `{ trackId }` | `bool` (there were some) |
| `lyrics_own_ids` | – | `string[]` |
| `lyrics_export` | `{ text, fileName }` | `string \| null` |

The expanded now-playing view's lyrics mode and the lyrics button use `lyrics_get`. Sources in order:
the track's Plex lyrics stream (a sidecar `.lrc` or `.txt` file, or Plex's
own lyrics), then lyrics the user wrote in the app, then LRCLIB when
`lyricsLrclib` is on. The user's own lyrics go first instead when they were
saved with `prefer` or `lyricsPreferOwn` is on (Plex isn't asked then).
LRCLIB is asked with
`/api/get` (artist, title, album, length in seconds); when that finds
nothing, `/api/search` with the artist and title only, taking a result only
when both match once case, accents and featured artists are folded away
(synced lyrics within 3 s of the track's length first, then synced within
10 s, then the closest length; synced lyrics further off keep their words
without the times). LRCLIB
answers are cached in SQLite (`lyrics_cache`): found 30 days, none 7 days.
A source that's down, busy or unreadable gives `null` and isn't cached. Only
a malformed id is an error (`invalid`).

Live versions: a track is live when its album's format or a subformat is
`live`, its ARCHIVE_VERSION is `live`, or the track or album title says so
("(Live)", "[Live ...]", "- Live", "Unplugged", and for albums "Live at",
"Live from", "Live in"). Lyrics found by artist and title (`plex` and
`lrclib`) are timed to some other recording, so on a live version they come
back with `borrowedTiming: true` and, while `lyricsLiveTiming` is off,
`synced: false` and no `timed`. A transcription of the song (shown at all)
goes ahead of them then. `file`, `user` and `auto` lyrics keep their timing.

`Lyrics = { synced, lines: string[], timed?: LyricLine[], credit?, source: "plex"|"file"|"user"|"lrclib", borrowedTiming? }`.
`source` is `file` when the Plex stream is a file next to the song (the
stream names Plex's local media agent, or no agent), `plex` when an agent
such as LyricFind supplied it, `user` for lyrics written in the app.
`lines` is the text in file order, `""` marking a break between verses.
`timed` is there when `synced`: lines in play order, `{ startMs, text, words?: { startMs, text }[] }`,
a line with several time tags appearing once per tag, `text: ""` marking an
instrumental gap. `words` comes from enhanced LRC `<mm:ss.xx>` tags or Plex
spans with their own offsets; joined, the words make `text`. An LRC
`[offset:±ms]` header is applied (positive shows lyrics sooner). `credit` is
the writer and provider line ("Lyrics from LRCLIB", or "Lyrics from LRCLIB,
matched by title" for a search match).

Own lyrics are what the lyrics editor saves, one row per track in SQLite
(`user_lyrics`), never written next to the music file.
`OwnLyrics = { trackId, text, prefer, updatedAt }`: `text` is LRC with one
`[mm:ss.xx]` tag per line when synced, plain lines otherwise, blank lines
between verses either way. `prefer` shows them ahead of Plex's for that song.
`lyrics_own_save` trims the text and refuses text with no words or over
64 KB, and a track id that isn't a library id (`invalid`). `lyrics_own_ids`
lets menus say "Add lyrics" or "Edit lyrics".

`lyrics_export` opens a save dialog (suggested name `fileName` made safe for
file systems, ending in `.lrc`) and writes `text` where the user picks.
Returns the saved file's name, or `null` when the dialog was cancelled.

## Lyrics transcription
| Command | Args | Returns |
| --- | --- | --- |
| `transcribe_status` | – | `TranscribeStatus` |
| `transcribe_model_download` | `{ model: "small"\|"medium" }` | – |
| `transcribe_model_cancel` | – | – |
| `transcribe_model_delete` | `{ model }` | – |
| `transcribe_track` | `{ trackId }` | – |
| `transcribe_album` | `{ albumId }` | songs queued (number) |
| `transcribe_unreleased` | – | songs queued (number) |
| `transcribe_cancel` | `{ trackId: string \| null }` | – |
| `transcribe_found_nothing` | `{ trackId }` | bool |

Events: `transcribe://status` → `TranscribeStatus` on every change (progress
at most four times a second), and `transcribe://done` → `{ trackId, found }`
when a song is finished (`found` false when no words came out).

Opt-in with `transcribeLyrics`. Speech recognition runs on this computer with
whisper.cpp; nothing about the song is sent anywhere. The only network use is
the model download, from the whisper.cpp model repository on Hugging Face,
pinned to one commit: `small` is `ggml-small.en-q5_1.bin` (190,098,681 bytes)
and `medium` is `ggml-medium.en-q5_0.bin` (539,225,533 bytes), both
English-only. The file goes to `<app data>/models/` as a `.part` file, is
checked against its SHA-256, and is renamed into place only when it matches.
Free space is checked first (the model plus 256 MB). Models never ship in the
installer.

`TranscribeStatus = { enabled, available, unavailableReason?, model, models,
current?, queued, batch, paused?, error? }`. `available` is false in a build
without the recognizer, or on an x86 processor without AVX2. `models` lists
`{ kind, sizeBytes, installed, downloading, doneBytes }` for each model, so
the size shows before downloading. `current = { trackId, stage, percent,
background }`, stage one of `waitingForModel`, `reading`, `decoding`,
`transcribing` (percent counts the last). `batch = { done, total }` counts
songs since the queue was last empty. `paused` is `battery`,
`slowConnection` or `offline` (background songs only). `error = { kind,
message, trackId? }` with kind `download`, `checksum`, `disk` or
`transcribe`; it clears when something new starts.

`transcribe_track` puts the song first and runs it now, battery or not.
`transcribe_album` queues the album's songs that have no lyrics in the app,
and `transcribe_unreleased` every unreleased song (the same rule as sharing:
demo release types or an unreleased `ARCHIVE_VERSION`) without them, in the
background. Before transcribing those, the core asks Plex (and LRCLIB when
it's on) and skips songs that already have lyrics. If the chosen model isn't
there, these start its download and the songs wait for it. All three fail
with `invalid` while `transcribeLyrics` is off; turning it off cancels the
queue, the song in progress and a download.

A song's file comes from the pre-cache when it's there; otherwise it's read
like the tag scan reads, through the player's transfer gate at the lowest
priority in 1 MB range requests, only while playback has nothing left to
download, never over the relay. It's decoded with the player's decoder,
mixed to mono and resampled to 16 kHz. Songs asked for now use about half
the processor cores; the background batch a quarter, and it waits while the
computer is on battery or music plays over remote access or the relay.

Words found are saved as enhanced LRC (a time per line and per word) in the
song's own lyrics (`lyrics_own_*`) with `auto` set to the model, so the
editor opens them. A new line starts at each recognizer segment, at a pause
of 0.7 s, or when a line runs long; sound descriptions such as `[Music]` and
stock phrases the recognizer invents over silence are dropped. A
transcription never replaces lyrics the user wrote or saved, and saving in
the editor clears `auto`. `lyrics_get` returns transcriptions with `source:
"auto"` after Plex's lyrics and before LRCLIB, never ahead of Plex even with
`lyricsPreferOwn`, and not at all while `showAutoLyrics` is off. Songs with
no words found are remembered (`transcribe_found_nothing`) so the batch
skips them.

## Downloads
| Command | Args | Returns |
| --- | --- | --- |
| `downloads_status` | – | `DownloadStatus` |
| `downloaded_tracks` | – | `string[]`, ids of every downloaded song |
| `downloads_set` | `{ kind: "album"\|"playlist"\|"liked"\|"mix", id, on: bool, title: string \| null, trackIds: string[] \| null }` | `DownloadStatus` |
| `downloads_retry` | `{ kind \| null, id \| null }` | – |
| `downloads_remove_all` | – | bytes freed (number) |
| `downloads_choose_folder` | – | the new folder, or null when the picker was cancelled |

`DownloadStatus = { items: DownloadItem[], usedBytes, limitBytes, freeBytes?, folder, paused?: "offline"|"notHome"|"full"|"noSpace", queued, current?: { trackId, received, total? } }`

`DownloadItem = { kind, id, title, state: "queued"|"downloading"|"downloaded"|"error", total, done, failed, missing, progress (0 to 1), bytes, error? }`

Events: `downloads://status` → `DownloadStatus` on every change (progress at
most four times a second), and `downloads://changed` (no payload) when the set
of downloaded songs changes; the UI asks `downloaded_tracks` again.

Whole albums, playlists, Liked Songs and mixes download, never single songs.
Albums, playlists and Liked Songs take their songs from the library cache and
follow it: right after every `library://changed` or `likes://changed` (live
library updates, playlist edits, likes), and every 30 seconds besides, new
songs queue and removed songs' files are deleted unless another downloaded
item still holds them. The item shows as downloading while it catches up and
the UI says "Downloading N songs", then "Download complete". Go offline, the
home network rule and the storage limit hold the new songs like any others. A mix has no list in the cache, so `downloads_set` for a mix needs
`trackIds`; calling it again with a new list updates it. Liked Songs' id is
ignored, and turning it on or off also sets `downloadLiked`.

Files are named by track id in the downloads folder (the app data folder's
`downloads`, or the one picked with `downloads_choose_folder`, which moves the
files). Original quality is the Plex part as it is, fetched with GET and
resumed with Range; an MP3 quality asks the universal transcoder and stops its
session afterwards. One file at a time goes through the player's transfer gate
at the lowest priority, after playback's own streams and the pre-cache. Album
covers are kept beside the files, so they show when the artwork cache has let
them go. A downloaded file plays before the pre-cache or the server, and counts
as on this computer for `cached_tracks` and `cached_library`.

The queue waits (`paused`) while the server can't be reached or `goOffline` is
on (`offline`), away from the home network while `downloadHomeOnly` is on
(`notHome`; home means the local connection to the server), at the storage
limit (`full`), or when the disk has no room (`noSpace`). Failed songs retry
after 30 seconds, 2 minutes, 10 minutes and an hour; after five tries the item
shows `error` until `downloads_retry`. A downloaded file deleted from disk is
noticed when it would play and at startup, and counts as `missing` until
retried. The queue survives restarts.

## Friend activity, following and listening sessions

| Command | Args | Returns |
| --- | --- | --- |
| `friend_activity` | `{ fresh: bool \| null }` | `ActivityPanel` (read again after 20 s, or now with `fresh`) |
| `follows_list` | – | `FollowedArtist[]` |
| `follow_set` | `{ id, title, thumb: string \| null, on: bool }` | `FollowedArtist[]`, the whole list |
| `whats_new` | – | `WhatsNew` |
| `whats_new_seen` | – | – |
| `listen_state` | – | `ListenView` |
| `listen_start` | – | `ListenView` with the first invite |
| `listen_invite` | `{ fresh: bool }` | `ListenView` with an unused invite (a new one when `fresh`, or when the last was used or ran out). Host only |
| `listen_stop` | – | – (ends the session you host, or leaves the one you're in) |
| `listen_join` | `{ link }` | `ListenView` (`connecting` until the host's first state arrives) |
| `listen_add` | `{ trackIds: string[] }` | – (a guest's go to the host) |
| `listen_control` | `{ op: ListenOp }` | – (a guest's go to the host; refused when the host only lets others add) |
| `listen_remove` | `{ personId }` | `ListenView`. Host only |

`ActivityPanel = { state: "ok"|"notAdmin"|"off"|"offline", friends: FriendActivity[] }`

`FriendActivity = { accountId, name, thumb?, trackId, title, artist, albumId?, album, cover?, at (unix s), now: bool }`

`FollowedArtist = { id, title, thumb?, followedAt (unix s) }`

`WhatsNew = { following: bool, items: { album: Album, new: bool }[], unseen }`

`ListenView = { role: "none"|"host"|"guest", sessionId?, hostName?, invite?: { link, qr?: { size, rows }, expiresAt (unix ms) }, people: { id, name, avatar?, host, me, cantHear }[], listeners: string[], guestsCanControl, nowBy?, queue: { trackId, by? }[], status?: "connecting"|"live"|"hostGone"|"ended"|"removed", relay: "connecting"|"ok"|"unreachable"|"limited", cantHear: string[], missing?: { title?, artist?, unreleased }, anotherVersion }`

For the host, `cantHear` on a person and the `cantHear` names say who has no
copy of the song playing. For a guest, `missing` says the host's song isn't
in this library (no title or artist when the host hides unreleased titles)
and `anotherVersion` that a copy of another length plays. A guest's `queue`
shows the copy here of each song that has one.

`ListenOp = { op: "play" } | { op: "pause" } | { op: "next" } | { op: "previous" } | { op: "seek", ms } | { op: "add", ids } | { op: "move", from, to }`

Events: `follows://changed` → `FollowedArtist[]`, `whatsnew://changed` →
`{ unseen }` (0 while What's New announcements are off), `listen://state`
→ `ListenView` whenever a session starts, someone joins or leaves, the
shared queue or the relay's state changes, or it ends, and `listen://notice`
→ `{ message }` when the host turns a guest's request down.

Friend activity reads the server's `/status/sessions` (live) and
`/status/sessions/history/all` for the music section (the newest 500 plays,
kept to the last 7 days), with names from `/accounts`. One row per person,
newest first; the viewer's own account is left out. Only the server owner's
token may read these, so anyone else gets `notAdmin`.

Follows are artist ids (a library artist, or a credited artist's `credit:`
id), kept in the sync store (the `follows` namespace, see Sync, folders and
keys), so they travel in the sync file when it's on. A list kept by an older
build moves over once at startup. What's New lists albums added
in the last 90 days by followed artists (credited artists bring the albums of
songs that credit them), or everything recently added while nothing is
followed, at most 60, newest first. `new` means added since `whats_new_seen`
was last called; the first look counts as seen. After each library sync,
albums added since the last check by artists followed before they were added
raise one desktop notification when `notifyNewReleases` is on and no private
session is.

### Listening sessions

Everyone in a session connects only to a relay, never to each other, so
nobody learns anyone else's address. The relay is an ntfy server: ntfy.sh
unless `listenRelay` names another. Nothing goes to it but random topic
names and sealed text. Guests play from their own access to the host's Plex
server, so songs travel as ids and the server as a hash of its machine id
keyed with the session id. No address, port, path, Plex token, machine id or
stream URL is ever sent.

Each song can also carry who it is, `{ mb?, n?, a?, d, u? }`: the
MusicBrainz track id, title, artist, length in ms, and whether it's
unreleased (the one release rule the app uses everywhere). For every song it's asked
to play, a guest uses the host's copy when its own library has that id,
else the same song in its library (the same MusicBrainz id, else the same
artist and title with lengths within 3 s), else none. A copy whose length
differs by more than 3 s plays from the same share of the song and shows as
another version. Unreleased songs are never matched (they go without a
MusicBrainz id), and with `listenHideUnreleased` on they go without a title
and artist too. With no copy the guest's player pauses, stays in the
session and picks up at the next song it has; it tells the host with
`cantHear` (the song's id, or none once it hears the session again). These
fields are optional: an older guest ignores them and an older host sends
none, so its guests play only its own copies. When the state is too big for
the relay, the identity of songs at the end of the queue goes first, then
the songs.

An invite is `platter://join/<token>`, the token being base64url of a
version byte, a random 16-byte session id, a random 16-byte invite secret,
the expiry (u32, unix seconds) and, only for a relay other than ntfy.sh, its
URL. Each invite works once and runs out after `listenInviteHours`.

Every direction of every conversation has its own topic (40 hex characters,
160 bits of an HMAC-SHA256 derivation) and key from a secret only the
people in it hold. A message is `1.<base64url(nonce, ciphertext)>`, sealed
with ChaCha20-Poly1305 with the topic as associated data. Inside is
`{ r: "h"|"g", f: <sender id>, c: <counter>, x: <expiry, unix ms>, m: <message> }`;
receivers drop anything expired or whose counter isn't above the last one
from that sender on that topic. Broadcasts from the host add
`.<base64url(tags)>`, one 128-bit HMAC tag per guest with a key only the
host and that guest share, so a guest can't pass a message off as the
host's.

Joining: the guest listens on the invite's down topic and sends `join` (a
nonce, its first SPAKE2 message over the invite secret, its name) on the up
topic. The first one the host sees gets `accepted`: the host's SPAKE2 answer
and, sealed with the key they agreed, the guest's id, its own secret, the
broadcast secret and the hashed server. Someone else holding the same link
can't open that, and gets `refused` with `used`, `expired` or `full`. A
guest on another server joins too, and plays its own copies.

Then each guest has its own up and down topics, and everyone listens on the
broadcast topic. The host sends `state` (song, position, length, playing,
up to 50 songs next with who added each, whether guests may change what's
playing, and everyone's names) when it changes and at least every 3
minutes, only while someone is listening. Guests send `hello` (send me the
state), `ping` every 3 minutes, `cmd` (a `ListenOp`), `cantHear` and
`leave`. The host
answers a turned-down `cmd` with `denied`. Removing a guest sends them
`removed`; a guest leaving or being removed gives everyone else a new
broadcast secret (`rekey`). Ending sends `ended` and forgets every key and
invite. A guest that hears nothing from the host for a heartbeat and 30 s
more shows `hostGone`, and goes back to `live` if the host comes back.

| Message | From | Topic |
| --- | --- | --- |
| `join`, `accepted`, `refused` | guest, host | the invite's up and down |
| `hello`, `ping`, `cmd`, `cantHear`, `leave` | guest | the guest's up |
| `state`, `rekey`, `removed`, `denied` | host | the guest's down |
| `state`, `ended` | host | broadcast |

ntfy as used: `POST <relay>/<topic>` with the sealed text and
`X-Firebase: no`, and `GET <relay>/<topic>,<topic>/json[?since=<id>]`, read
line by line. A 429 shows as `limited`, anything else that fails as
`unreachable`; both retry with a growing wait.

## Smart playlists, hide, bookmarks and scrobbling

| Command | Args | Returns |
| --- | --- | --- |
| `smart_preview` | `{ rules: SmartRules, picks: string[] \| null }` | `SmartPreview` |
| `smart_save` | `{ request: SmartSaveRequest }` | `SmartSaved` |
| `smart_list` | – | `SmartSummary[]` (Platter smart playlists) |
| `smart_page` | `{ id }` (a `smart:` id) | `SmartPage` |
| `smart_delete` | `{ id }` (a `smart:` id) | – |
| `smart_plex_rules` | `{ id }` (a Plex smart playlist) | `SmartRules`, or `invalid` when its rules don't map to the editor |
| `smart_prompt` | `{ prompt }` | `{ title, rules: SmartRules, picks: Track[] }` |
| `ai_status` | – | `{ provider: "none"\|"anthropic", model, hasKey, ready }` |
| `ai_key_set` | `{ key: string \| null }` | the same, after saving or removing the key |
| `hidden_list` | – | `HiddenItem[]`, newest first |
| `hidden_set` | `{ item: { kind: "track"\|"artist", id, title, subtitle?, thumb? }, on: bool }` | `HiddenItem[]` |
| `bookmarks_list` | – | `Bookmark[]`, newest first |
| `bookmark_add` | `{ bookmark: { kind: "track"\|"page", target, page?, title, subtitle?, thumb?, positionMs? } }` | `Bookmark` (the one already there for the same song second or page) |
| `bookmark_remove` | `{ id }` | `Bookmark[]` |
| `bookmark_open` | `{ id }` | `Bookmark`; a song bookmark plays from its time first |
| `scrobble_status` | – | `ScrobbleStatus` |
| `lastfm_secret_set` | `{ secret: string \| null }` | `ScrobbleStatus` |
| `lastfm_sign_in_start` | – | `ScrobbleStatus`, after opening Last.fm's approval page in the browser |
| `lastfm_sign_in_finish` | – | `ScrobbleStatus` (signed in), or `invalid` until the user approves |
| `lastfm_sign_out` | – | `ScrobbleStatus` |
| `listenbrainz_check` | – | `ScrobbleStatus` (asks ListenBrainz whose token it is) |

`SmartRules = { matchAll, rules: { field, op, value, plexField? }[], limit?: { unit: "songs"|"hours", value }, sort, update: "live"|"daily"|"weekly", extra?: [string, string][] }`.
Fields: `genre`, `style`, `mood` (is, isNot), `artist`, `album`, `version`
(is, isNot, contains, notContains), `year`, `playCount`, `rating` (stars),
`duration` (minutes) (is, isNot, gt, lt), `decade` (is, isNot, a year like
1990), `addedAt`, `lastPlayed`, `playedHere` (inLast, notInLast, with `30d`,
`2w`, `6m` or `1y`), `liked`, `explicit` (is, `"true"` or `"false"`) and
`releaseType` (is, isNot, a release section key). Sorts: `none`, `random`,
`mostPlayed`, `leastPlayed`, `recentlyAdded`, `recentlyPlayed`, `rating`.
`extra` keeps Plex options the editor doesn't show (one copy per song, other
sorts) so saving a Plex playlist back doesn't lose them.

`SmartPreview = { count, durationMs, tracks: Track[] (the first 50), plexGaps: string[] }`.
The count is worked out on this computer from the library cache, with the
picks first. `plexGaps` says why the rules need a Platter smart playlist;
empty means Plex can hold them.

`SmartSaveRequest = { id?, title, summary?, rules, picks?: string[], prompt?, platter?: bool }`
and `SmartSaved = { id, platter, note? }`. A new rule set Plex can express is
created as a Plex smart playlist (see docs/PLEX-API.md "Smart playlists") and
read back into the cache, so it shows with the other playlists. Rules Plex
can't express (`playedHere`, `duration`, `releaseType`, `explicit`,
`version`, a limit in hours, a daily or weekly update), prompt picks, or
`platter: true` make a Platter smart playlist with a `smart:` id. When Plex
refuses a rule set it's saved as a Platter one and `note` says so. Offline, a
Plex one can't be made and the answer is a `network` error. Editing a Plex
smart playlist (`id` set) keeps it in Plex; rules it can't hold are refused.

`SmartSummary = { id, title, summary?, trackCount, durationMs, update, nextUpdateAt? (unix s), updatedAt, prompted }`
and `SmartPage = { playlist: SmartSummary, rules, picks, prompt?, tracks }`.
Platter smart playlists live in the sync store (`smartPlaylists`
namespace). A live one is filled again each time its page opens; a daily or
weekly one keeps its songs on this computer until the next whole day or week.
A random sort holds for the same period. The library lists them as smart
playlists with "Platter" where the owner goes; the playlist route with a
`smart:` id opens their page.

`smart_prompt` needs `aiProvider` set and a key saved. It asks the provider
twice: once with the description and the library's most used genre, style
and mood names and its years, for rules; then with up to 150 songs those
rules find (title, artist, album, year; unreleased music left out), for the
best 50 in order. Both answers are JSON in a fixed shape. The editor shows the
rules and picks before anything is saved.

`HiddenItem = { kind, id, title, subtitle?, thumb?, hiddenAt (unix s) }`.
Hidden songs and artists (an artist hides every song whose album artist it
is) are skipped by mixes, radio, "keep playing similar music" and shuffle
play (`play_context` with `shuffle: true`, except the song picked to start).
Picked directly, or played in order, they still play. A mix page leaves out
songs hidden after it was made. They're kept in the sync store's `hidden`
namespace, keyed `track:<id>` or `artist:<id>`.

`Bookmark = { id, kind, target, page?, title, subtitle?, thumb?, positionMs?, createdAt (unix s) }`.
`page` is a route name: `album`, `artist`, `playlist`, `smart`, `mix`,
`radio`, `liked` or `discography`. Kept in the sync store's `bookmarks`
namespace, keyed by id.

`ScrobbleStatus = { lastfm: { hasKey, hasSecret, signedIn, user?, rejected, pending }, listenbrainz: { hasToken, user?, rejected }, queued }`.
A song counts when it's longer than 30 s and was heard for half its length or
4 minutes, whichever is less (seeks don't count). Each service on, signed in
and not refusing the sign-in gets "now playing" when a song starts and the
scrobble when it ends, with the time it started. Ones that can't go (offline,
the service down, a refused sign-in) wait on this computer, at most 5000 and
two weeks old, and go out oldest first, 50 at a time, once a minute or right
after a new sign-in. A private session sends nothing and queues nothing.
Unreleased music is left out unless `scrobbleUnreleased` is on. Last.fm
signs in with its desktop flow: a token, the user approves Platter on
last.fm, then a session key. The API key (shared with artist info), its
shared secret and the session key are in the OS keychain, as is the
ListenBrainz token.

Events: `hidden://changed` → `HiddenItem[]`, `bookmarks://changed` →
`Bookmark[]`, `smart://changed` → `SmartSummary[]` (also
`library://changed`), and `scrobble://status` → `{ queued }` when the queue
or a sign-in changes. Changes another device made through the sync file send
the same events.

## Listening stats

| Command | Args | Returns |
| --- | --- | --- |
| `stats_overview` | `{ range: { from, to }, limit? }` | `StatsOverview` |
| `stats_span` | – | `{ first, last, months: string[], today }` |
| `stats_item` | `{ kind: "song"\|"artist"\|"album", id }` | `ItemStats` |
| `stats_month_recap` | `{ month }` ("YYYY-MM") | `MonthRecap` |
| `stats_play_ids` | `{ range, kind: "songs"\|"artists"\|"albums"\|"genres", limit? }` | track ids, most played first |
| `stats_compare_summary` | `{ unreleasedOk, theirUnreleasedOk }` | `CompareSummary`, or `invalid` while Compare is off |
| `stats_compare` | `{ theirs: CompareSummary, unreleasedOk }` | `CompareResult`, or `invalid` while Compare is off |

Stats count every play that wasn't a skip, from the listen log and imported
plays together. A range is two local dates ("YYYY-MM-DD"), both included; no
dates means all time. Plays are summed per local day for each song, artist,
album and genre, and per day and hour. Changing, adding or removing a play
marks its day, and the next stats call counts only the marked days again.

`StatsOverview = { from, to, totals: { ms, plays, songs, artists, albums, days }, bucket: "day"|"month", series: { key, plays, ms }[], streak: { days, from, to } | null, hours: number[24], weekdays: number[7], busiestHour, busiestWeekday, topSongs, topArtists, topAlbums, topGenres }`.
The chart is by day for ranges under 92 days and by month for longer ones.
Weekdays start on Sunday. Top lists rank by plays, then minutes. A top song
is `{ key, trackId, title, artist, album, plays, ms, unreleased, track }`;
`track` is the library song when it's still there.

`ItemStats = { kind, plays, ms, first: { at, song, trackId, from: { kind, id, title } | null } | null, lastAt, rank, series }`.
`rank` is the place in your all-time top for that kind, `series` the plays
per month for the last 12 months.

`CompareSummary = { version, name, artists: { name, weight }[], songs: { artist, title, weight }[], unreleasedOk }`
is all that's shared with a friend: up to 50 artists and 50 songs from about
the last six months, each with its share of the plays. Unreleased songs are
only in it when both sides agreed. `CompareResult = { friend, overlap (0 to 100), commonArtists, theirSongsNew }`.

## Lidarr

Lidarr's home and away addresses are the `lidarrUrl` and `lidarrAwayUrl` settings. The API key and any extra proxy headers are in the OS keychain and never cross back to the UI. The check only reads from Lidarr. Requests send exactly three kinds of write: `POST /api/v1/artist` (monitored, but with `monitorNewItems` and `addOptions.monitor` set to `none` and no search), `PUT /api/v1/album/monitor` and `POST /api/v1/command` with `AlbumSearch`. Before any of them the release group is checked on MusicBrainz again and the whole setup check runs again; a red finding stops the request.

| Command | Args | Returns |
| --- | --- | --- |
| `lidarr_status` | – | `{ urlSet, awaySet, hasKey, headerNames: string[], unencrypted: string[], ready }`; `ready` is true when Lidarr is set up with a key and its last check for this address passed |
| `lidarr_key_set` | `{ key: string \| null }` | the same, after saving or removing the key |
| `lidarr_headers_set` | `{ text: string \| null }` (`Name: value` lines; blank removes) | the same; `invalid` for a bad line, a reserved header or more than 5 |
| `lidarr_check` | – | `LidarrReport`, or `network` (unreachable), `auth` (key refused), `invalid` (too old, not Lidarr, not set up), `decode` |
| `lidarr_confirm_root` | `{ path, confirmed }` | – (the user says an empty root folder is separate from the Plex music folder) |
| `lidarr_pick` | `{ artist, artistId?, title, song: boolean, isrc? }` | `{ group: { id, title, primary, secondary, firstRelease }\|null, artist, refusal, reason }`: the release group to request for an album or song, or why there is none |
| `lidarr_request_preview` | `{ input: LidarrRequestInput }` | `LidarrPreview`; reads only. `invalid` for a red check, a refused release group (bootleg, promo, someone else's, a title already in the library) or "Lidarr doesn't know this album"; `server` when Lidarr's metadata service is down |
| `lidarr_request` | `{ input: LidarrRequestInput }` | `LidarrRequest`. Kept waiting (`state: "queued"`) when Lidarr can't be reached; the first request ever needs `typed` to be the album's name |
| `lidarr_requests` | `{ refresh: boolean }` | `LidarrRequest[]`, newest first; `refresh` reads their state from Lidarr's queue and history |
| `lidarr_request_cancel` | `{ id }` | `LidarrRequest[]`; only waiting or finished requests, else `invalid` |
| `lidarr_missing_releases` | `{ artistId, artist }` | `{ shown, items: { releaseGroupId, title, kind: "album"\|"ep", year, request, thumb }[] }`; `thumb` is the small front cover as an art path (`/ext/https://coverartarchive.org/release-group/<id>/front-250`), which fails to load when the archive has none; `shown` is false unless the setting is on, Lidarr is set up and its last check passed |

`LidarrReport = { version, findings: { id, level: "red"|"amber"|"ok", title, detail, fix: string|null, emptyRoot: { path, confirmed }|null }[], passed, checkedAt, route: "home"|"away"|null }`.
Findings come red first, then amber, then OK. `passed` is false while any finding is red.
The home address is tried first (for 4 seconds when an away address is set), then the away address.

`LidarrRequestInput = { releaseGroupId, title, artist, artistId?, typed? }`.
`LidarrPreview = { releaseGroupId, title, artist, destination, destinationIsGuess, needsTyped, existing, reachable, route, addsArtist }`.
`LidarrRequest = { id, releaseGroupId, title, artist, artistId, state, destination, lidarrAlbumId, addedArtist, alreadyInLidarr, createdAt, updatedAt, message, notes }`, with `state` one of `queued`, `requested`, `searching`, `downloading`, `imported`, `failed`.
Requests are kept on this device. Waiting ones are retried every 5 minutes in the background, full check first.

## Sync, folders and keys

| Command | Args | Returns |
| --- | --- | --- |
| `sync_file_status` | – | `SyncFileStatus` |
| `sync_file_now` | – | `SyncFileStatus` after a pass |
| `sync_file_folders` | – | `CloudFolder[]`, cloud folders found on this computer |
| `sync_file_use_folder` | `{ path }` | `SyncFileStatus` (sets `syncFolder`, turns `syncEnabled` on) |
| `sync_file_choose_folder` | – | `SyncFileStatus`, or null when the picker was cancelled |
| `sync_file_replace` | – | `SyncFileStatus` (an unreadable file is moved aside, a new one written) |
| `pins_get` | – | `Pin[]`, newest first, 12 at most |
| `pins_toggle` | `{ pin: Pin }` | `Pin[]` |
| `pins_import` | `{ pins: Pin[] }` | `Pin[]` (taken only into a store that never had pins) |
| `folders_get` | – | `Folders` |
| `folder_create` | `{ name: string \| null, parent: string \| null }` | `Folder` ("New Folder" without a name) |
| `folder_rename` | `{ id, name }` | – |
| `folder_delete` | `{ id }` | – |
| `folder_move` | `{ id, parent: string \| null }` | – |
| `playlist_folder_set` | `{ playlistId, folderId: string \| null }` | – |
| `keys_available` | – | `KeyKind[]`, the keys this computer has |
| `keys_receive_start` | – | `ReceiveState` with a new code |
| `keys_receive_state` | – | `ReceiveState` |
| `keys_receive_cancel` | – | – |
| `keys_send` | `{ code, kinds: KeyKind[] }` | `{ device, sent: KeyKind[] }` |

`SyncFileStatus = { state: "off"|"signedOut"|"ok"|"folderMissing"|"unreadable"|"newer"|"error", folder?, file?, lastSyncedAt? (ms), devices: { name, lastSeen (ms), thisDevice }[], mergedCopies, message? }`

`CloudFolder = { provider: "OneDrive"|"iCloud Drive"|"Google Drive"|"Dropbox", path }`

`Pin = { kind: "album"|"artist"|"playlist", id, title, subtitle, thumb?, pinnedAt? (ms) }`

`Folders = { folders: { id, name, parent?, createdAt (ms) }[] (newest first), items: { [playlistId]: folderId } }`

`KeyKind = "listenbrainz"|"lastfm"|"fanart"|"audiodb"|"imgur"|"catbox"` (`catbox` carries the `discordUploadCovers` choice; the name is kept so older copies understand it)

`ReceiveState = { phase: "idle"|"waiting"|"receiving"|"received"|"expired"|"locked", code?, qr?: { size, rows: string[] }, expiresAt? (ms), from?, received: KeyKind[] }`

Events: `sync-file://status` → `SyncFileStatus` when it changes,
`syncstore://changed` → `{ namespaces: string[], remote: bool }` after any
change to the store (`remote` when it came from the sync file), and
`keys://receive` → `ReceiveState`.

### The sync store

App-only state lives in the `sync_entries` table, one row per entry: a
namespace, a key, a JSON value (none for a deletion), when it changed and on
which device. The namespaces are `settings` (one entry per synced setting),
`pins`, `folders`, `folderItems` (playlist id to folder id), `follows`,
`hidden` (keyed `track:<id>` or `artist:<id>`), `bookmarks` (by bookmark id),
`lyrics` (lyrics written in the editor, by track id) and `smartPlaylists`
(Platter smart playlists, synced with the library folders). Without a sync file this is simply where the state is kept.

Features use the typed store in `src-tauri/src/sync/store.rs`:
`store.put(&FOLLOWS, id, &value)`, `store.get`, `store.list`, `store.remove`,
`store.write_many` for several changes at once, and `store.subscribe()` for
changes. Values are refused when a field name looks like a secret (token,
password, secret, API key, client ID), so keys can't end up in the file. The
followed artists go through the social module's `FollowStore`
(`sync::follows::SyncFollows`).

### The sync file

With `syncEnabled` and a folder, Platter keeps `Platter/sync-<tag>.json` in
that folder, where the tag is the hash of the Plex account id that Platter
devices announce with, so each Plex account has its own file. A pass reads
the file and any conflicted copies a cloud app left beside it
(`sync-<tag> (conflicted copy).json`, `sync-<tag>-PC.json` and the like),
takes every entry that wins into the store, adds this device's entries that
win, writes the file back when anything changed, and deletes the copies. The
later change wins per entry; on the same millisecond the larger device id
does. A deletion stays as a tombstone for 90 days. Entries of groups turned
off in What to sync are neither sent nor taken, and other devices' entries
for them stay in the file. The file also lists each device's name and when it
last wrote.

Writes go to a temp file beside the file, then a rename, so a cloud app never
picks up half a file. A pass runs 2 s after the last change here, when the
file's size or time changes (checked every 3 s), every 60 s and on
`sync_file_now`. While the folder can't be reached the changes stay in the
store and go out with the next pass that can write. A file that isn't a sync
file Platter can read is left alone (`unreadable`) until `sync_file_replace`
moves it aside as `sync-<tag>.broken-<ms>.json`; one written by a newer
Platter (`newer`) is never overwritten. Folder detection only looks: it never
creates anything until a folder is chosen.

### Library folders

Plex has no folders, so these are Platter's own. `folder_create` makes "New
Folder" at once. Deleting a folder moves its playlists and folders up to
where it was; playlists are never deleted with a folder. A folder can't move
into itself or a folder inside it, and folders nest 8 deep at most. After a
merge, a folder whose parent is gone shows at the top, a loop of parents is
broken at the folder with the smallest id, and a playlist placed in a missing
folder shows at the top.

### Copying keys

Keys never go through the sync file. `keys_receive_start` shows a 6-digit
code (and a QR code of `platter://copy-keys/<code>`) for 5 minutes. On a
computer signed in to the same Plex account, `keys_send` asks the user's
other Platter devices on the control port which one is showing a code, runs
a SPAKE2 exchange (Ed25519 group) over the code with it, checks that the
receiver reached the same key, then sends the chosen keys sealed with
ChaCha20-Poly1305 under a key made from the exchange, with both device ids
bound in. The receiver stores each key through the feature that uses it, so
it lands in the OS keychain. A wrong code fails the exchange and uses up one
of 5 tries per code; after that the code stops working (`locked`). The
control port requests are signed with the account key like the others.

| Route on the control port | Body | Answer |
| --- | --- | --- |
| `GET /platter/v1/keys/waiting` | – | `{ waiting }` |
| `POST /platter/v1/keys/hello` | `{ msg (hex), name }` | `{ msg (hex), confirm (hex) }`, 404 when no code is showing |
| `POST /platter/v1/keys/deliver` | `{ nonce (hex), sealed (hex) }` | `{}` |

## Search
| Command | Args | Returns |
| --- | --- | --- |
| `search` | `{ query, filter: "all"|"songs"|"artists"|"albums"|"playlists", limit? }` | `SearchResults { top?: SearchHit, songs: Track[], artists: Artist[], albums: Album[], playlists: Playlist[], tookMs, source: "local"|"server" }` |
| `search_completions` | `{ query, limit? }` | `string[]` |
| `search_recents` | – | `SearchHit[]` |
| `search_recent_add` | `{ kind, id }` | – |
| `search_recent_remove` | `{ kind, id }` | – |
| `search_recents_clear` | – | – |

`SearchHit = { kind: "artist"|"album"|"track"|"playlist", artist?, album?, track?, playlist? }`

Until the first library sync has finished, `search` asks the server instead
(`/hubs/search`), ranks the answer with the same rules and returns
`source: "server"`. If the server is slow or unreachable it answers from the
cache. After the first sync, search never touches the network.

With filter `all`, `limit` applies to every kind; without it, All returns
four songs and ten of each other kind.

`search_completions` returns up to `limit` (default 4, at most 10) whole
artist, album and track names that start with the typed words, most played
first, without duplicates. It only reads the local index.

Recent searches are the items you opened from search, not query text.
`search_recent_add` puts an item at the top (an item already there moves up)
and keeps the newest 20. `search_recents` skips items that are no longer in
the library. Settings `searchSuggestions` and `searchRecents` turn the
dropdown parts off; the UI checks them.

## Playback and queue
| Command | Args |
| --- | --- |
| `play_context` | `{ context: { kind: "album"|"playlist"|"artist"|"tracks"|"search"|"liked"|"mix"|"radio", id?, trackIds? }, startTrackId?, shuffle? }` |
| `play_toggle`, `play_pause`, `play_resume`, `play_next`, `play_previous` | – |
| `play_seek` | `{ positionMs }` |
| `set_volume` | `{ volume: 0..1 }` (linear slider value; core applies the perceptual curve) |
| `set_muted` | `{ muted }` |
| `set_shuffle` | `{ shuffle }` (for the playing context) |
| `set_context_shuffle` | `{ contextId, shuffle }` (applies now if that context is playing, otherwise the next time it plays) |
| `set_smart_shuffle` | `{ on }` for the playing playlist or Liked Songs (on also turns shuffle on; off turns shuffle off and keeps the order) |
| `set_context_smart_shuffle` | `{ contextId, on }` (applies now if it is playing, otherwise the next time it plays) |
| `smart_suggestion_remove` | skips the playing suggestion and picks new ones |
| `smart_suggestion_keep` | `{ itemId }` the playing suggestion was added to its playlist or liked; it becomes a playlist song |
| `set_smooth_transitions` | `{ on }` (crossfade for the playing context only; ends when another context starts) |
| `set_repeat` | `{ repeat: "off"|"all"|"one" }` |
| `set_ab_loop` | `{ abLoop: { aMs, bMs } | null }` (null clears it) |
| `queue_add` | `{ trackIds, at: "next"|"end"|{ index } }` (`index` puts them at that spot in the user queue; past the end appends) |
| `queue_remove` | `{ itemId }` |
| `queue_move` | `{ itemId, toIndex }` (index within the user queue) |
| `queue_clear` | – (clears the user queue only) |
| `queue_jump` | `{ itemId }` (play that item now) |
| `player_state` | – → `PlayerState` |

`PlayerState = { status: "idle"|"loading"|"playing"|"paused"|"buffering"|"error", track?: Track, positionMs, durationMs, volume, muted, shuffle, shuffledContexts: string[], repeat, queue: QueueView, abLoop: { aMs, bMs } | null, smartShuffle, smartContexts: string[], smoothTransitions }`
`QueueView = { current?: QueueItem, userQueue: QueueItem[], nextUp: QueueItem[], history: QueueItem[], context?: { kind, id?, title? } }`
(the window gets at most the next 500 songs in `nextUp`, so playing a whole library doesn't send every song on each change)
`QueueItem = { itemId, track: Track, source: "user"|"context"|"autoplay"|"smart" }` (`autoplay`: similar music after the context ran out; `smart`: put in by smart shuffle)

Shuffle is remembered per album, playlist or artist id; repeat is one global
setting. `shuffle` is the playing context's state and `shuffledContexts` lists
every context id whose shuffle is on, so a page can show its own toggle for a
context that isn't playing. `play_context` without `shuffle` uses that
context's stored value; passing `shuffle` sets it.

Queue rules the core applies:
- Without `startTrackId`, an album or playlist (Liked Songs and mixes too)
  starts at its resume point (the track and position it was left at) when
  shuffle is off, at track 1 if it was never played, and on a random track
  when shuffle is on. A point expires an hour after the context was last
  listened to, and is cleared when its last song plays to the end (pausing
  near the end keeps it). Shuffled listening leaves the point alone. Starting
  a chosen track moves the point there. Artists, radio and search results
  never resume. Resume points and shuffle values are saved with the queue,
  for the 200 most recently used contexts.
- `play_previous` within the first 3 s goes to the previous position in the
  context's play order (the shuffled order when shuffled). Queued songs are
  never a target; on a playing queued song it drops that song and goes back
  to the context track before it. On the first track, or at 3 s or later, it
  restarts the track.
- In repeat-one, `play_next` or an early `play_previous` switches repeat to
  all. A late `play_previous` restarts and keeps repeat-one.
- When the last song of a context starts with repeat off and `queueEnd` is
  `keepPlaying`, 20 similar songs are added after it (`source: "autoplay"`),
  so playback carries on. With `stop`, or when nothing similar is found, the
  player goes back to the first track when the context runs out, paused at
  0:00, with the rest of the context as upcoming.
- A radio context asks for 20 more songs whenever fewer than 5 are left,
  never repeating one already in the queue or played lately.
- Smart shuffle works on playlists and Liked Songs only, while the
  `smartShuffle` setting is on. When the context starts (or smart shuffle is
  turned on) the core builds a fixed list: the shuffled songs with one
  suggestion (`source: "smart"`) after every two of them, counting the playing
  song, and never after the last one. It isn't refilled as songs are used up.
  Suggestions are library songs in the playlist's style by artists who aren't
  in it, songs that aren't liked first. Removing the playing suggestion skips
  it and builds new ones. Turning smart shuffle off keeps the playing song,
  drops the suggestions still to come and plays the rest in the order it was
  shuffled into. The choice is remembered per context id (`smartContexts`) with
  the queue, so it survives restarts. When the list runs out, keep playing adds
  similar music as usual (`source: "autoplay"`).

A-B loop: `set_ab_loop` takes the two points in either order and swaps them so
A comes first. They're clamped to the track and must be at least 1 s apart,
otherwise the call fails with `invalid` and the loop is left as it was. The
core does the looping, so it keeps going with the window hidden: when playback
reaches B (or the end of the audio) it goes back to A. After one full pass
from A to B the core keeps that audio and replays it, so later passes have no
gap; loops longer than about a minute re-read from A each time instead. The
loop belongs to the playing track. Any track change, a seek outside A..B, or
turning off `abLoopHandles` clears it. Pausing, resuming and seeking inside
the span keep it.

Events:
- `player://state` → `PlayerState` (on any state/track/queue change)
- `player://position` → `{ positionMs, durationMs, bufferedMs }` (4×/s while playing)
- `player://error` → `{ trackId?, kind, message }` (e.g. "Skipped: file unavailable")

## Mixes and radio
Made on this computer from the cache and the listen log; nothing is stored
on the server except playlists saved from a mix (`playlist_create`).

| Command | Args |
| --- | --- |
| `mixes_home` | `{ tzOffsetMin? }` → `MixShelves` (makes whatever is due first) |
| `mix_page` | `{ id, steer?: "familiar"|"new", tzOffsetMin? }` → `MixPage` (a steered mix is made again, not stored) |
| `mix_refresh` | `{ id }` → `MixPage` (that mix made again with new songs, and stored) |
| `radio_page` | `{ seed: "track:<id>"|"artist:<id>", steer?, salt? }` → `MixPage` (about 50 songs, a song radio starting with its song; the same `salt` gives the same songs) |

`tzOffsetMin` is the UI's time zone in minutes east of UTC; the core keeps the
last one for its own checks.

`MixShelves = { madeForYou: MixSummary[], moods: MixSummary[], genres: MixSummary[] }`
`MixSummary = { id, kind: "daily"|"discover"|"newInLibrary"|"onRepeat"|"rediscover"|"genre"|"mood"|"daylist"|"topSongs"|"radio", title, description, color, art: string | null, covers: string[], artistIds: string[], tag: string | null, trackCount, durationMs, nextUpdateAt: number | null }`
`MixPage = { mix: MixSummary, tracks: Track[], familiar: boolean[], steer: "familiar"|"new" | null }`

What each mix is:
- `daily:1` to `daily:6`: your most played artists grouped by shared Plex
  genres, styles and moods, songs played in the same session, and Plex
  similar artists. About 70% favourites from the group and 30% less played
  songs from it and its close neighbours (the `mixVariety` setting moves the
  split), 50 songs. A song is in one Daily Mix a day at most.
- `discover`: 30 songs never played, closest to what you play most, two per
  artist at most.
- `new`: songs from albums added in the last 30 days.
- `onRepeat`: the 30 songs played most in the last 30 days (two plays or more).
- `rediscover`: songs played three times or more, none in the last six months.
- `topSongs`: All-Time Top Songs, the 100 most played by play count.
- `genre:<name>` and `mood:<name>`: one per tag with at least 25 songs, 12 of
  each at most, the tags you play most first.
- `daylist`: songs you play around this hour on this kind of day (weekday or
  weekend), named "<genres and moods> <weekday> <part of day>", made again
  every `daylistHours`. `nextUpdateAt` says when.

Skips under 30 s count against a song and plays for it. No artist plays more
than twice in a row. The explicit filter, hidden fan edits, `mixUnreleased`
and the exclusion lists apply to every mix and to radio. Mixes are made again
at the first check after 4 am local time each day, when a mix setting
changes, and on demand; the daylist when its block of hours changes. All of it
runs off the UI thread.

Events:
- `mixes://changed` → `null` (mixes were made again)

## Output devices
| Command | Args |
| --- | --- |
| `output_devices` | – → `OutputState` |
| `output_select` | `{ deviceId: string \| null }` → `OutputState` (null follows the system default) |

`OutputState = { devices: OutputDevice[], selected: string | null, current: OutputDevice | null, exclusive: Exclusive }`
`OutputDevice = { id, name, isDefault }`
`Exclusive = { state: "off" } | { state: "on", rate, bits } | { state: "fallback", reason }`

`exclusive` says how the open stream holds its device. It's `on` while
bit-perfect output (below) has the device to itself at `rate` Hz with `bits`
significant bits, and `fallback` when bit-perfect was asked for but the
device is playing shared, with a plain `reason`.

`devices` is every output the system reports, in its order: virtual devices
included, inputs and disconnected devices left out. The device in use is
always in it. `id` is the OS's stable device id. `selected` is the chosen device, or null
when playback follows the system default. `current` is the device playing, or
the one the next play would open. Switching moves the open stream at once and
keeps the position and play state. The choice is saved as the
`outputDevice` setting (this device only); `settings_set` doesn't change it.
An unknown id is a `notFound` error.

While following the system default, a change of default moves playback to
the new device without stopping. If the device in use is unplugged, playback
pauses and Play resumes on whatever output is there. If that device was the
chosen one, the choice drops back to the system default for this session
and `output://lost` fires; the saved setting is kept, so the device is used
again at the next start if it's back.

Events:
- `output://changed` → `OutputState` (after a switch, a fallback, or a reroute)
- `output://lost` → `{ device }` (name of the chosen device that went away)

## Listen on
The user's other computers running Platter, Plex players Platter can move
playback to (Plexamp, Plex apps, Sonos linked to Plex), Chromecasts and
AirPlay speakers on the local network, and remote mode while one plays.

| Command | Args |
| --- | --- |
| `remote_players` | – → `RemotePlayer[]` (finds them now; other Platter computers always, Plex players only with `showPlexPlayers`, Cast and AirPlay devices only with `showCastDevices`) |
| `remote_move_with_password` | `{ playerId, password }` → `RemoteState` (AirPlay speakers with a password; the password goes to the keychain once it works) |
| `remote_state` | – → `RemoteState` |
| `remote_move` | `{ playerId }` → `RemoteState` |
| `remote_return` | – → `RemoteState` |
| `remote_command` | `{ command: RemoteCommand }` → `RemoteState` |
| `remote_play_context` | `{ context, startTrackId? }` → `RemoteState` (same `context` as `play_context`) |
| `remote_queue_jump` | `{ itemId }` → `RemoteState` |

`RemotePlayer = { id, name, product, kind, playing: string | null, online, via, needsPassword, unsupported: string | null, unreachable }`
with `kind` one of `"computer" | "phone" | "speaker" | "browser" | "tv" | "group"`
and `via` one of `"platter" | "plex" | "cast" | "airplay"`. Cast ids start with `cast:`,
AirPlay ids with `airplay:`; Platter computers use their Plex client id.
The list comes in this order: Platter computers (reachable first), then Plex
players, then Cast and AirPlay devices. `unreachable` is set on a Platter
computer that announces itself or that Plex lists as on, but that doesn't
answer directly (another network, or a firewall on that computer); the
picker explains that once. `needsPassword` is set for an AirPlay speaker
with a password that isn't in the keychain yet; the picker asks for it and
calls `remote_move_with_password`. `unsupported` says why a listed device
can't be played on (for example an AirPlay speaker that only takes encrypted
or paired senders); moving there fails with an `invalid` error.
`playing` is the grey line from the server's sessions ("Song • Artist", or
"Paused: Song • Artist"). `online` is false for players plex.tv knows but
that aren't on the network now; they're listed last and may not answer.

`RemoteState = { session: RemoteSession | null, stoppedOn: string | null }`
`RemoteSession = { playerId, playerName, kind, status, track: Track | null, positionMs, durationMs, volume: number | null, shuffle, repeat }`
with `status` one of `"connecting" | "playing" | "paused" | "buffering"`.

`RemoteCommand` is tagged by `kind`: `play`, `pause`, `next`, `previous`,
`{ kind: "seek", positionMs }`, `{ kind: "volume", volume }` (0..1),
`{ kind: "shuffle", on }`, `{ kind: "repeat", mode }`.

`remote_move` makes a server play queue from the current song and the rest
of the queue (up to 300 songs), tells the player to play it at the current
position, pauses here (unless `moveStopsOldDevice` is off) and starts
following the player once a second. If the player doesn't take it, the
error is a `network` error with the message "Couldn't reach <device>" and
playback here carries on. Moving from one player to another comes back here
first.

While a session is open the player bar shows the session over the local
state and its controls use `remote_command`; playing an album or a queue
entry uses `remote_play_context` or `remote_queue_jump`, which load it here
paused and send the new queue to the same player.

The queue here stays on what the device has coming up, so `player_state`'s
queue (and the queue panel) shows the device's real next songs. For a Plex
player its server play queue is read again whenever the timeline's
`playQueueItemID`, `playQueueVersion` or song changes, and every 10 seconds
in case the player doesn't report a version; the queue here then moves along
to that song (paused) or takes the play queue as it is. For another Platter
or a Cast device the queue here moves along as its song changes. With
`moveStopsOldDevice` off the queue here is left alone.

`queue_add`, `queue_remove`, `queue_move` and `queue_clear` also reach the
device while a session is open. A Plex player's play queue is edited in place
(added, moved and removed entries) and the player is told to read it again,
so the song there keeps playing. Another Platter or a Cast device gets the
whole queue again from its song and spot. If the play queue doesn't take it,
the command fails with a `network` error "Couldn't update the queue on
<device>". AirPlay plays this computer's queue, so edits apply as they are.

`remote_return` reads the player's position, stops it (unless
`moveStopsOldDevice` is off) and carries on here: the same song seeks, a song
further along the queue is jumped to, and a song outside the queue plays
with the player's play queue. It plays if the player was playing.

If the player stops answering (3 polls in a row), a toast says "Couldn't
reach <device>" and playback here is put on its last song and position,
paused. If the player reports stopped after it has played (someone stopped
it there), the session ends the same way without a toast and `stoppedOn`
names the device until the next move or return.

Chromecasts (and Cast speaker groups) are found over mDNS
(`_googlecast._tcp`). A move connects to the device, starts Plex's Cast
receiver and loads the same server play queue with a transient token from
the server (`/security/token?type=delegation&scope=all`), so the device
streams from the server and plays the queue by itself. If Plex's receiver
doesn't start, the default media receiver gets up to 100 songs as stream
URLs instead (formats Cast can't play go through the server's MP3
transcoder). Volume is the device's own. The session ends with `stoppedOn`
when another app or sender takes the device over.

AirPlay speakers are found over mDNS (`_raop._tcp`). A move keeps the queue
and decoding here and moves the audio output to the speaker (Apple Lossless
at 44.1 kHz over RAOP); the output id becomes `net:airplay:<id>` and the
local volume goes to full while the speaker's volume follows the slider.
What's heard runs about two seconds behind, so `positionMs` is the heard
position; pause, seek and skip clear the speaker's buffer so they take
effect at once. A wrong or missing password is an `auth` error ("This
speaker needs its AirPlay password." or "That AirPlay password didn't
work."); a speaker that only takes paired AirPlay 2 senders is an `auth`
error saying so. Coming back puts the output and volume back. If the
speaker drops off, the output falls back to this computer paused (the usual
lost-device path) and `stoppedOn` names it.

AirPlay receivers that belong to a computer are never listed: a Mac model in
the `am` TXT key, no model with video and screen mirroring in `ft`, or a
receiver at the address of one of the user's Platter computers. Receivers
that need a password or a pairing code are listed only with
`showAirplayLocked` on (one whose password is already in the keychain still
shows).

### Platter to Platter

With `showToMyDevices` on (the default) and Plex signed in, each Platter:
- announces `_platter._tcp` over mDNS with the TXT keys `id` (its Plex client
  id), `name` (`computerName`, or the OS computer name when empty),
  `platform`, `version` and `acct` (16 hex digits of
  SHA-256("platter-account-v1\n" + the Plex account uuid));
- listens on TCP port 32520 (or the next free one up to 32529) on its
  private LAN IPv4 addresses only (RFC 1918 and link-local; never loopback,
  100.64/10 or public addresses);
- publishes those addresses to plex.tv every 5 minutes
  (`PUT /devices/<client id>?Connection[][uri]=http://<address>:<port>`), so
  plex.tv resources list it with `lastSeenAt`.

Devices are listed only when `acct` matches. A Platter that plex.tv lists
(seen in the last 15 minutes, or online) but that doesn't answer is shown
with `unreachable`. One plex.tv has an old address for and hasn't seen
lately is left out.

The control API is JSON over HTTP, one request per connection:
`GET /platter/v1/hello` (id, name, platform, version, playing),
`GET /platter/v1/state` (state, timeMs, durationMs, trackId, volume 0..1,
shuffle, repeat), `POST /platter/v1/play` (serverId, current, userQueue,
nextUp, context, positionMs, shuffle, repeat, play), `POST
/platter/v1/command` (a `RemoteCommand`) and `POST /platter/v1/stop`
(pauses). Every request carries
`X-Platter-Auth: v1 from=<sender id>;ts=<unix ms>;nonce=<hex>;sig=<hex>`,
where `sig` is HMAC-SHA256 with the key SHA-256("platter-peer-key-v1\n" +
the sender's own Plex token) over the method, path, sender id, receiver id,
time, nonce and the SHA-256 of the body. Every device has its own token, so
the receiver looks the sender id up in the account's device list from
plex.tv (`/api/v2/devices`, fetched with its own token, cached 5 minutes and
fetched again for a sender it can't vouch for, at most every 10 s) and
checks the signature with the key of the token listed there. No token is
sent between devices.

The receiver refuses (401) with JSON `{error, message, auth: 2, now}`:
`unsigned`, `notYourAccount` (not on the device list, or signed with another
token: a different account or Home user), `cantCheck` (no device list from
plex.tv yet), `clock` (more than 2 minutes from its clock; `now` is its
time), `replayed` (a nonce it has seen) and `signedOut`. A sender refused
for the clock signs once more with the receiver's time. Versions before
`auth: 2` refused every other device with `notYourAccount`; the sender says
that one needs updating.

`remote_move` to a Platter computer sends its queue as it is (the current
song, the user queue and next up in play order, the context, shuffle,
repeat and position) to `/platter/v1/play`; the receiver plays from its own
connection to the same server and refuses a different one ("That computer
plays from a different Plex server."). No server play queue is made. From
there it works like a Plex player: the sender pauses, shows "Playing on
<name>", follows `/state` once a second, sends controls to `/command`, and
`remote_return` reads the position and sends `/stop`. When the computer
doesn't answer directly but is registered with the server as a Plex player
(its `actAsPlexPlayer` is on), the move goes through the Plex companion path
instead. A refused request is an `auth` error that says why, for example
"Couldn't play on <name>. It's signed in to a different Plex account or Home
user.", and the picker row shows the same reason.

Events:
- `remote://state` → `RemoteState` (on every change and every poll while remote)
- `remote://players` → `RemotePlayer[]` (after each refresh)

The player list refreshes when `remote_players` is called (the picker does
every 10 s while open) and every 60 s in the background while a session is
open.

With `actAsPlexPlayer` on, the core listens on the first free port from
32500 and answers the Plex companion calls (`/resources`,
`/player/timeline/poll`, `/player/playback/playMedia`, play, pause, stop,
skipNext, skipPrevious, seekTo, setParameters). It announces itself with GDM
on the local network. Media from another server is refused.

## Health and app states
| Command | Returns |
| --- | --- |
| `health` | `Health` |
| `cached_tracks` | `string[]` (ids of tracks stored on this computer) |
| `cached_library` | `{ albums: Album[], artistIds: string[], playlistIds: string[] }`, everything with a song stored on this computer (albums most recently played first) |
| `open_local_network_settings` | `null`. macOS only: opens System Settings at Privacy & Security, Local Network, and checks the connection again |

`Health = { status: "local"|"remote"|"relay"|"offline"|"starting"|"updating"|"noAuth", serverName?, connection?, latencyMs?, serverVersion?, lastSeenAt? (Unix ms), scan?: { done, total }, banner?: { kind, message }, filesAvailable: bool, cachedOnly: bool }`

`banner.kind` is one of `offline`, `noInternet`, `starting`, `updating`,
`startTimeout`, `relay`, `signIn`, `accessRemoved`, `filesUnavailable`,
`noMusicLibrary`, `diskFull`, `localNetwork`, `goneOffline`; `message` is the text to show.
`offline`, `noInternet` and `goneOffline` show as the top bar's offline icon, not a banner. `scan` counts new
tracks while Plex scans ("Adding new music… 120 of 340"). `cachedOnly` is true
while only music already on this computer can play; the UI then asks
`cached_tracks` and dims the rest. See docs/APP-STATES.md.

Event `health://changed` → `Health`. Event `toast` → `{ kind: "info"|"error", message }`.

## Themes
| Command | Args | Returns |
| --- | --- | --- |
| `themes_list` | – | `ThemeFolder` |
| `themes_import` | – | `ThemeFile`, or null when the file picker was cancelled. The file is checked first and copied into the folder under its theme name |
| `themes_open_folder` | – | – (shows the folder in Explorer or Finder) |

`ThemeFolder = { folder: string, themes: ThemeFile[], invalid: { file, reason }[] }`.
`ThemeFile = { file, name, base: "dark" | "light", tokens: Record<string, string> }`.

Theme files are JSON in a `themes` folder in the app's data folder:

```json
{ "name": "Harbour", "base": "dark", "tokens": { "color-bg-base": "#08121A", "radius-card": "12px" } }
```

A file has `name`, `base` and `tokens`, and may have `author`. Tokens start
from the base's built-in theme. Only the token names in
`src/design/themes/contract.json` are taken (with or without a leading `--`).
Colour tokens take `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`,
`hsl()` or `hsla()`; the radius tokens take a pixel size like `8px`. Files
over 64 KB, unknown fields or tokens, `url()`, CSS (`;`, braces, comments,
`var()`) and markup are refused, and the file is listed under `invalid` with
the reason. The UI applies the same checks again before using a theme.

## Settings and tracking
| Command | Args |
| --- | --- |
| `settings_get` | – → `Settings { privateSession, reportToPlex, volume, shuffleStyle, ... }` |
| `settings_set` | `{ patch: Partial<Settings> }` |
| `art_cache_info` | – → `{ usedBytes, capBytes }` |
| `art_cache_clear` | – → bytes freed (number) |

`privateSession` (default false) stops plays being written to the local
listen log. `reportToPlex` (default true) controls timeline and scrobble
reports; when it's off nothing is sent or queued. `volume` is the same 0..1
value as `set_volume`; setting it here also moves the player. `shuffleStyle`
is `"spread"` (default: keeps an artist's songs apart and pushes recently
played ones back) or `"standard"` (plain random order). Settings are
stored in the local database and apply immediately.

The rest of the fields, with their defaults:

| Field | Values | What it does |
| --- | --- | --- |
| `qualityHome` | `"original"`, `"320"`, `"256"`, `"192"`, `"128"` | Bitrate cap on the local connection. Original plays the file as is. |
| `qualityAway` | same, default `"320"` | Cap over remote access and the relay. The relay never goes above 128 kbps. |
| `showFormatBadge` | bool, true | Shows the codec and sample rate badge in track lists and album headers. |
| `resumeContexts` | bool, true | Albums and playlists start where they were left. Off starts at track 1; points are still saved. |
| `precacheCount` | 1 to 50, 5 | Upcoming tracks kept on disk. Out of range values are clamped. |
| `popularSource` | `"global"`, `"myPlays"` | Ranks the artist page's Popular by the global top list (ListenBrainz or Last.fm) or by your own plays. |
| `artCacheMb` | 128 to 51200, 2048 | Artwork cache size. Shrinking it removes the oldest images at once. |
| `artworkColours` | bool, true | Headers tint from artwork. Off gives every header the same neutral colour. |
| `reduceMotion` | `"system"`, `"on"`, `"off"` | Follow the OS setting, or force reduced motion on or off. |
| `theme` | `"dark"` (default), `"light"`, `"system"`, `"midnight"`, `"forest"`, `"sand"` | The theme. System follows the computer's light or dark mode. A theme file in `themeFile` takes its place. See Themes. |
| `accent` | string, `"ivory"` | Accent colour: `ivory`, `sage`, `sky`, `lilac`, `amber`, `rose`, `teal`, or a custom colour as `#RRGGBB` (stored in capitals). Anything else is refused. Ivory is each theme's own accent. |
| `themeFile` | `ThemeFile` or null | The theme file in use, held whole so it travels in the sync file. Checked like a file in the folder; a bad one is refused. Null clears it. |
| `outputDevice` | device id or null, null | Read only here. Set with `output_select`; null follows the system default. |
| `homeShortcuts` | bool, true | Home shows the grid of recent and most played shortcuts. |
| `homeColourWash` | bool, true | Home's top takes its colour from the shortcuts' artwork. Only applies while `artworkColours` is on. |
| `searchSuggestions` | bool, true | Show completions and direct results under the search box while typing. |
| `searchRecents` | bool, true | Show items opened from search when the empty search box is focused, and remember new ones. |
| `mediaSession` | bool, true | Show the current track in the system media controls (the Windows media flyout and lock screen, macOS Now Playing) and take play, pause, next, previous and seek from them. |
| `mediaKeys` | bool, true | Hardware media keys control Platter in the background. While `mediaSession` is on the OS sends the keys to that session; with it off, Platter registers the keys itself. |
| `closeToTray` | bool, false | Shows a tray icon (menu bar item on macOS). Closing the window hides it instead of quitting. Turning it off brings a hidden window back. |
| `launchAtLogin` | bool, false | Starts Platter at sign-in, hidden in the tray when `closeToTray` is on, otherwise minimised. |
| `abLoopHandles` | bool, true | Right-click or Alt+click on the seek bar sets A-B loop points. Turning it off clears a loop that's set. |
| `upNextLine` | bool, true | In the last 15 s of a track (last 30 % under 50 s) the player bar's artist line shows what's next. |
| `volumePercent` | bool, true | Shows the volume percentage above the slider while it changes. |
| `notifyTrackChange` | bool, false | Desktop notification when a new track starts. Not shown while the window has focus, in a private session, or when the previous track change was under 5 s ago. Clicking it brings the window forward. This device only. |
| `playlistAskDuplicates` | bool, true | Ask before adding a song that's already in the playlist. |
| `playlistNewName` | text, `"My playlist #N"` | Name for new playlists; `#N` becomes the next free number. Up to 100 characters. |
| `infoWikipedia` | bool, true | Artist info from Wikipedia and Wikidata. |
| `infoLastfm` | bool, false | Artist info from Last.fm. Only used while a key is saved and wasn't refused. |
| `infoFanart` | bool, false | Artist images from Fanart.tv. Same key rule. |
| `infoAudiodb` | bool, false | Artist photo and bio from TheAudioDB. Same key rule. |
| `infoListenbrainz` | bool, true | Popular's global list from ListenBrainz (Last.fm top tracks fill in when it has none and Last.fm is on). With this and `infoLastfm` both off, Popular ranks by your plays. The optional token is kept in the keychain. |
| `infoMusicbrainz` | bool, true | MusicBrainz: album release dates and credits, artist members with their years, related artists. Synced. |
| `infoListenbrainzStats` | bool, true | ListenBrainz listener numbers when Last.fm has none. Synced. |
| `linkFeatured` | bool, true | Every name in a track's artist line links to a page, featured artists without a library entry included, and search lists them. Off leaves those names as plain text. |
| `likedMeans` | `"five"` or `"fourUp"`, `"five"` | Which Plex ratings count as liked: five stars (10), or four stars and up (8 or more). Liking always writes five stars. |
| `likeControlRows` | `"hover"`, `"always"` or `"off"`, `"hover"` | When song rows show the like control. Liked songs always show their tick unless it's `"off"`; the player bar and now playing always show it. |
| `showPlexPlayers` | bool, true | List Plex players under Listen on in the device picker. Off makes `remote_players` return an empty list. |
| `moveStopsOldDevice` | bool, true | Moving playback to a player pauses it here, and coming back stops the player. Off leaves both playing. |
| `actAsPlexPlayer` | bool, false | Platter answers as a Plex player so other Plex apps can play here. Opens a local port. This device only. |
| `showCastDevices` | bool, true | List Chromecasts, Cast groups and AirPlay speakers under Listen on. Off stops mDNS browsing. |
| `crossfade` | `"smart"`, `"fixed"`, `"off"`, `"smart"` | How one song moves into the next. See Sound below. |
| `crossfadeSeconds` | 1 to 12, 6 | Length of a fixed crossfade. |
| `crossfadeSkipAlbums` | bool, true | Songs of one album played in album order never crossfade. |
| `leveling` | `"smart"`, `"track"`, `"album"`, `"off"`, `"smart"` | Loudness leveling with Plex's gain values. |
| `preampDb` | -12 to 12, 0 | dB added to every track. This device only. |
| `limiter` | bool, true | Soft limiter so boosts never clip. This device only. |
| `eqEnabled` | bool, false | The equalizer. This device only, like the rest of the EQ. |
| `eqBands` | 10 numbers, -12 to 12, all 0 | The active curve at 31, 62, 125, 250, 500 Hz, 1, 2, 4, 8 and 16 kHz. Setting it makes the preset `"custom"` unless it matches the selected one. |
| `eqPreset` | preset id, `"profile:<name>"` or `"custom"`, `"flat"` | Setting it to a preset or profile also sets `eqBands`. Presets: `flat`, `bassBoost`, `trebleBoost`, `vocal`, `loudness`, `smallSpeakers`, `lateNight`. |
| `eqProfiles` | `{ name, bands }[]`, empty | Read only here. Saved curves; change them with the `eq_profile_*` commands. |
| `eqOutputs` | `{ deviceId, deviceName, preset }[]`, empty | Read only here. The preset or profile each output device uses. Set with `eq_use_for_output`. |
| `eqFallback` | `{ preset, bands }` or null | Read only. The choice for devices without their own, kept from the last such device. |
| `eqLastOutput` | device id or null | Read only. The device the EQ last followed. |
| `headphoneCorrection` | `{ deviceId, deviceName, name, source, form, path, preampDb, filters }[]`, empty | Read only here. The headphone correction each output device uses. Set with `headphone_correction_set`. This device only. |
| `bitPerfectOutputs` | `{ deviceId, deviceName }[]`, empty | Read only here. Output devices that play bit-perfect. Set with `bit_perfect_set`. This device only. |
| `shareCardQr` | bool, true | Share cards include a QR code that opens the item in Platter (the `platter://` link only). |
| `shareCardSongLink` | bool, true | Share cards for released songs and albums include the song.link line. Also needs `sharePublicLinks`. |
| `sharePublicLinks` | bool, true | Share menus find public links for released songs and albums (iTunes Search and song.link). Off leaves only Platter links and sends nothing. |
| `discordStatus` | bool, true | Show what's playing on Discord. This device only. See Discord above. |
| `discordClearOnPause` | bool, true | Pausing clears the status. Off shows the song as paused, without a progress bar. |
| `discordShowUnreleased` | bool, true | Off shows nothing while an unreleased song plays. |
| `discordUnreleasedArt` | bool, true | Off shows the app icon for unreleased songs; their cover is never looked up or uploaded. |
| `discordFindSong` | bool, true | The Find this song button. Also needs `sharePublicLinks`. |
| `discordCoverArchive` | bool, true | Covers from Cover Art Archive by MusicBrainz release id, or by a MusicBrainz search on album artist and title for released albums. |
| `discordUploadArt` | bool, false | Upload covers Cover Art Archive lacks to Imgur. Only used while a client ID is saved and wasn't refused. |
| `discordUploadCovers` | bool, false | Upload covers Cover Art Archive lacks (mostly unreleased songs) to keyless image hosts. Uploads are public to anyone with the link and can't be deleted. Saved as `discordUploadCatbox` by earlier builds, which loads into this. This device only. |
| `discordCoverHost` | `"auto"\|"litterbox"\|"x0"\|"uguu"\|"catbox"`, `"auto"` | Where covers are uploaded: automatic (Litterbox, x0.at, uguu.se, catbox.moe in that order) or one host. This device only. |
| `discordStatusLine` | `"app"\|"artist"\|"song"`, `"app"` | What the member list shows after "Listening to" (`status_display_type`). |
| `showPlexAvatar` | bool, true | The app menu button shows the Plex profile picture (`AuthStatus.avatar`). Off shows the first letter of the username. This device only. |
| `libraries` | string[], [] | Libraries included besides the connected server's main one, as `<machineId>/<sectionId>` (see More than one library). Changing it starts a sync. This device only. |
| `serverView` | string, "all" | What the library shows with more than one server: "all" or a server's tag. This device only. |
| `lyricsMode` | `"autoShow"\|"off"`, `"autoShow"` | Auto-show opens the expanded view in lyrics mode when a song with synced lyrics starts while now playing is open. Off shows lyrics only from the lyrics button. |
| `lyricsLrclib` | bool, false | `lyrics_get` asks LRCLIB when Plex has no lyrics (sends artist, title, album and length; the search sends artist and title). Off sends nothing. |
| `lyricsPreferOwn` | bool, false | Lyrics written in the app show ahead of Plex's for every song that has them. Off shows them only when Plex has none, or for songs saved with `prefer`. |
| `lyricsLiveTiming` | bool, false | Live versions keep the timing of lyrics found by artist and title (still marked `borrowedTiming`). Off shows them unsynced. |
| `lyricsShowAdd` | bool, true | The expanded view's lyrics mode offers Add lyrics for a song with none. The track menu keeps Add lyrics either way. |
| `transcribeLyrics` | bool, false | Transcribe songs without lyrics on this computer (Transcribe this song, the album menu, the unreleased batch). See Lyrics transcription. This device only. |
| `transcribeModel` | `"small"` or `"medium"`, `"small"` | The speech model. Medium is more accurate and about three times slower. This device only. |
| `showAutoLyrics` | bool, true | `lyrics_get` returns transcriptions the user hasn't edited. Off hides them; they stay saved. |
| `fullWindowHideControls` | bool, true | The expanded view fades its header after 3 s without mouse movement or a key. |
| `readFileTags` | bool, true | Read explicit, version and edition tags from the files in the background. Off stops the scan; tags already read stay. This device only. See File tags above. |
| `explicitContent` | `"allowed"` or `"filtered"`, `"allowed"` | Filtered leaves songs tagged explicit out of every context that plays, with one toast per context. |
| `showFanEdits` | bool, false | Off leaves fan and AI edits (`ARCHIVE_FLAGS`) out of album pages and what plays, and hides albums made only of them. |
| `countUnmatchedUnreleased` | bool, true | Albums Plex couldn't match (guid `local://`, or no release type and no label before the next sync) count as unreleased for sharing, Discord, scrobbling, mixes, smart playlists and lyrics transcription until a release type or `ARCHIVE_VERSION` says otherwise. Off: only demo release types and archive version tags count. |
| `checkUnmatchedMusicbrainz` | bool, true | Look albums Plex couldn't match up on MusicBrainz in the background (needs `infoMusicbrainz`). A sure match means Plex missed it and the album counts as released. Off, or with MusicBrainz off, stored answers stop counting. Synced. |
| `groupEditions` | bool, true | Albums sharing `ARCHIVE_GROUP` list as one album (the face) in the library, artist pages and Home's album shelves. |
| `plexEditShow` | bool, true | Edit in Plex on album and artist menus, for servers the account owns. Off refuses every edit. |
| `plexEditLock` | bool, true | Fields changed with `plex_edit_details` are locked in Plex so a refresh keeps them. |
| `plexEditMatchUnreleased` | bool, true | Fix match is offered on unreleased albums (always with a warning). Off refuses it there. |
| `dragAndDrop` | bool, true | Songs, albums and playlists can be dragged onto playlists, Liked Songs and the queue. Off stops those drags; reordering inside a playlist or the queue still works. |
| `showToMyDevices` | bool, true | Announce this computer to the user's other Platter computers, open the control port and publish its addresses to plex.tv. Off withdraws the announcement and closes the port. Independent of `actAsPlexPlayer`. This device only. |
| `computerName` | string, `""` | The name other devices see, up to 60 characters, trimmed. Empty uses the OS computer name. This device only. |
| `showAirplayLocked` | bool, false | List AirPlay receivers that need a password or a pairing code. Computers' own receivers are never listed. This device only. |
| `downloadQuality` | `"original"`, `"320"`, `"192"` or `"128"`, `"original"` | Original keeps the file as it is; the others are MP3s from the server's transcoder. Applies to new downloads. This device only. |
| `downloadLimitGb` | number, 20 | Downloads stop at this size, 1 to 4096. This device only. |
| `downloadFolder` | string, `""` | Where downloads go; empty is the app data folder. Changed with `downloads_choose_folder`, not `settings_set`. This device only. |
| `downloadHomeOnly` | bool, true | Downloads wait while the server is reached over remote access or the relay. This device only. |
| `downloadLiked` | bool, false | Keep Liked Songs downloaded; new likes download too. The same as the Liked Songs download toggle. This device only. |
| `homeMixes` | bool, true | Home shows Made for you, Your mixes by mood and Your genre mixes. |
| `mixVariety` | `"balanced"`, `"familiar"` or `"adventurous"`, `"balanced"` | The share of favourites in a mix: 70%, 85% or 50%. |
| `mixExcludeArtists` | string[], `[]` | Artist ids never in mixes or radio. |
| `mixExcludeGenres` | string[], `[]` | Genre, style or mood names (any case) never in mixes or radio. |
| `mixExcludeAlbums` | string[], `[]` | Album ids never in mixes or radio. |
| `mixUnreleased` | bool, true | Demos and archive versions marked unreleased can be in mixes and radio. |
| `daylistHours` | 1, 3 or 6, 3 | Hours between daylist refreshes. |
| `queueEnd` | `"keepPlaying"` or `"stop"`, `"keepPlaying"` | When the queue ends, keep playing similar music, or rewind and stop. |
| `smartShuffle` | bool, true | On playlists and Liked Songs the shuffle button has a third state, smart shuffle. Off ends smart shuffle that's on. |
| `goOffline` | bool, false | No network use: no requests to the server or plex.tv, no artist info, LRCLIB, share link lookups, Discord status, Listen on discovery or update checks. Health shows `offline` with the `goneOffline` banner and `cachedOnly`, so only music on this computer plays. Off reconnects as at startup. This device only. |
| `showFriendActivity` | bool, true | The top bar's friend activity button and panel. |
| `shareActivity` | bool, true | Send Plex now-playing timelines, so friends on the server see you live. Off (and in a private session) none go out; finished plays still reach Plex's history. |
| `whatsNewAnnouncements` | bool, true | The bell's dot for unseen What's New items. |
| `notifyNewReleases` | bool, false | A desktop notification when a followed artist gets new music in the library. This device only. |
| `listenGuestsControl` | bool, true | "Let others change what's playing": guests in a session you host can play, pause, skip, seek and reorder. Off, they can only add songs. |
| `listenInviteHours` | 1, 24 or 168, 24 | How long an invite works (each works once). |
| `listenName` | string, `""` | The name others in a listening session see; empty is your Plex name. 40 characters at most. |
| `listenRelay` | string, `""` | The listen along relay, an ntfy server URL; empty is ntfy.sh. A URL of your own goes in your invites. |
| `listenHideUnreleased` | bool, false | In a session you host, unreleased songs go to guests without their title and artist. Synced. |
| `discordListenAlong` | bool, true | Discord shows a listening session you host and how many are in it. This device only. |
| `syncEnabled` | bool, false | Keep app-only state in a sync file in `syncFolder`. On with no folder counts as off. This device only. See Sync, folders and keys. |
| `syncFolder` | string, `""` | The folder the sync file goes in. Changed with `sync_file_use_folder` or `sync_file_choose_folder`, not `settings_set`. This device only. |
| `syncSettings` | bool, true | What to sync: settings that aren't this device only. This device only. |
| `syncLibrary` | bool, true | What to sync: library folders and pins. This device only. |
| `syncFollows` | bool, true | What to sync: followed artists, hidden and snoozed items, bookmarks. This device only. |
| `syncLyrics` | bool, true | What to sync: lyrics written in the editor. This device only. |
| `bookmarksInLibrary` | bool, true | A Bookmarks row under Liked Songs in the library. |
| `scrobbleLastfm` | bool, false | Scrobble to Last.fm once signed in. This device only. |
| `scrobbleListenbrainz` | bool, false | Scrobble to ListenBrainz with the user token. This device only. |
| `scrobbleUnreleased` | bool, false | Unreleased music scrobbles too. |
| `aiProvider` | `"none"` (default), `"anthropic"` | The AI provider for prompted playlists. None hides them. This device only (the key is in this computer's keychain). |
| `aiModel` | string, `"claude-opus-5-5"` | The provider's model. Letters, numbers, dots, dashes; empty goes back to the default. |
| `miniplayerOnTop` | bool, true | The miniplayer window stays above other windows. This device only. |
| `miniplayerTint` | bool, true | The miniplayer's card takes a colour from the cover. This device only. |
| `miniplayerQueue` | bool, false | The miniplayer lists the next songs when it's at least 480 tall. This device only. |
| `miniplayerBounds` | `{ x, y, width, height }` or null, read only | Where the miniplayer was last, in logical pixels. Kept by the core. This device only. |
| `powerKeys` | bool, false | Keyboard power mode in the main window: jump hints, j / k, g g, G, Enter and /. Handled in the UI (`src/shell/powerKeys.ts`); the core only stores it. |
| `hintKey` | `"f"` (default), `"h"`, `";"`, `"'"` | The key that shows the jump hints. |
| `nowPlayingFeed` | bool, false | Serve the playing song on 127.0.0.1 for widgets (see Now-playing feed). This device only. |
| `nowPlayingFeedPort` | 1024 to 65535, 32540 | The feed's port. Lower ports are refused with `invalid`. This device only. |
| `allowStreamDeck` | bool, false | Let a paired Stream Deck control playback on 127.0.0.1 (see Stream Deck). This device only. |

Quality changes apply from the next track. Settings saved by an older build
load with these defaults for anything missing.

Settings belong to the Plex Home user Platter runs as (each has their own
data folder). One setting is for the whole device and isn't a field here:
Remember the last Home user (default on), read with `home_users` and set
with `home_remember_set` (see Home users).

Settings marked This device only stay on this computer. Every other one
travels in the sync file while sync is on and `syncSettings` is. The two
lists are `SYNCED_SETTINGS` and `DEVICE_SETTINGS` in `settings.rs`, and a test
checks every setting is in exactly one. A synced setting changed on another
computer arrives as a `settings_set` patch would, and the UI hears
`settings://changed` with the whole `Settings`.

The last four only touch the core's platform layer (`src-tauri/src/platform/`).
It follows the player through the same state and position events the UI gets
and sends OS requests back as ordinary player commands, so they add no
commands or events. Cover art goes to the OS as a local file copied from the
art cache, never as a server URL.

## Sound
| Command | Args |
| --- | --- |
| `eq_profile_save` | `{ name }` → `Settings`. Saves the current curve under `name` (replacing a profile with that name) and selects it. Built-in names are refused. |
| `eq_profile_rename` | `{ from, to }` → `Settings`. Outputs using it keep it. |
| `eq_profile_delete` | `{ name }` → `Settings`. Outputs using it lose their binding; if it was selected the curve stays as `"custom"`. |
| `eq_use_for_output` | `{ on }` → `Settings`. Binds the selected preset or profile to the output device in use (`on: false` unbinds it). A custom curve has to be saved first (`invalid`). |
| `headphone_search` | `{ query }` → `HeadphoneEntry[]` (at most 50). Headphones in the bundled correction index whose name has every word of `query`, ignoring case and accents. |
| `headphone_correction_set` | `{ path: string \| null }` → `Settings`. Uses that headphone's correction whenever the output device in use plays; null turns it off there. The profile is fetched the first time (`network`, `notFound`, `decode` errors) and kept on disk. |
| `headphone_correction_compare` | `{ bypass }` → –. Plays without the correction while true, to compare. Not saved. |
| `bit_perfect_set` | `{ on }` → `Settings`. Bit-perfect output on or off for the output device in use; the stream reopens at once, keeping position and play state. |

`HeadphoneEntry = { name, source, form, path }`: `source` is who measured
it (and on what rig when they used several), `form` is earbud, in-ear or
over-ear, and `path` is its place in the index.

Everything runs in the core between decode and output, so it works with the
window hidden. The chain is leveling gain, crossfade mix, headphone
correction, EQ, limiter,
volume.

Crossfade. Fixed fades the outgoing song out and the next one in over the
set time with equal-power curves (never more than half of either song).
Smart measures each song's quiet lead-in and run-out (50 ms windows under
-40 dBFS) while it decodes, remembers them per track, and overlaps only those:
the overlap is the outgoing run-out plus the incoming lead-in, at most 8 s,
and only those quiet parts are faded, so a song that ends cold plays to its
last sample and a song that fades out keeps its own fade. There is no
crossfade on next, previous or a queue jump (those cut with the short ramp as
before), on an A-B loop wrap, between album tracks in order while
`crossfadeSkipAlbums` is on, or when the next song isn't decoded far enough
yet (the change is gapless then). The outgoing song's `Ended` and the next
song's start happen when the fade begins.

Leveling. Plex's `gain` and `albumGain` (dB) and `peak` and `albumPeak` are
read from track detail the first time a track plays and kept in the library
cache. Smart uses album gain while an album plays in album order and track
gain otherwise; Track and Album always use that one (falling back to the
other when it's missing). Gain is applied as Plex gives it. A track with no
gain data gets only the preamp. Gains are set before a track is heard;
changing a setting mid-track ramps over 200 ms.

EQ and limiter. Ten octave-wide peaking bands at the output sample rate.
Band changes ramp over 200 ms; a flat EQ costs nothing. The limiter looks
1.5 ms ahead and keeps peaks at -0.5 dBFS; its gain work is skipped when the
peaks Plex reports, the gain and the EQ curve can't reach that. Turning it on
or off dips the sound for about 10 ms.

Headphone correction. Profiles come from the AutoEq project (MIT). The app
ships a small index of names and folders; a profile's parametric EQ text is
fetched from raw.githubusercontent.com (the only host allowed) when someone
picks it, and cached under the app's cache folder. Peaking, low shelf and
high shelf filters are used, with the profile's preamp (never above 0 dB,
and worked out from the biggest boost when a file has none), so a
correction can't clip. It runs before the EQ. Switching profiles while
playing dips the sound for about 10 ms.

Bit-perfect output. On Windows the device opens in WASAPI exclusive mode;
on macOS it's taken in hog mode with its sample rate set to the music's.
Each song plays at its own sample rate (the stream reopens between songs at
different rates, so those aren't gapless), and the samples go out as
decoded: volume, EQ, headphone correction, leveling, preamp, limiter and
crossfades are all bypassed. Mute still works. If the device is busy, its
exclusive use is turned off in the system settings, or it can't play the
rate, playback carries on shared with `exclusive.state` `fallback` and a
reason. Streams Plex transcodes are converted to the device's rate, so
they're played exclusively but aren't bit-perfect.

When the output device changes (a switch, a new system default, an unplug),
the core switches to the preset or profile bound to the new device, or back
to the earlier choice on a device without one, saves it and sends
`settings://changed`. The device's headphone correction and bit-perfect
choice follow it the same way.

Events:
- `settings://changed` → `Settings` (the core changed a setting itself)

## Updates
| Command | Returns |
| --- | --- |
| `update_check` | `{ available: bool, version?, notes? }` |
| `update_install_on_restart` | – |
| `app_info` | – → `{ version, licence, repoUrl }` |
| `open_project_page` | – (opens the project page in the default browser) |

Event `update://ready` → `{ version }`.
