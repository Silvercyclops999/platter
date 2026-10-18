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

Event `home://changed` → `{ restarting: bool }` just before the restart
(also when signing out as another Home user restarts the app). The core then
waits 250 ms so the UI can cover the window with its "Switching to..."
screen, and the new window opens where the old one was, shown even if the
app was launched at login (see Main window). The UI puts that screen up as
soon as a user is picked. When `home_switch` fails (a wrong PIN, plex.tv out
of reach) nothing has changed: the screen goes and a toast says why. If the
restart hasn't happened 20 seconds after the pick, the screen goes too.

`auth_status` waits up to 2 seconds at launch for the saved sign-in to be
read from the keychain (never for the network), so the first answer is not
a signed-out one.

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

### Servers
| Command | Args | Returns |
| --- | --- | --- |
| `backend_caps` | – | `BackendCaps { caps: Caps, servers: ServerCaps[] }` |
| `servers_saved` | – | `SavedServer[]` |
| `server_set_primary` | `{ tag }` | – |
| `server_remove` | `{ tag }` | – (servers added by address; Plex signs out in Account) |
| `server_libraries` | `{ tag }` | `{ id, title, included }[]` |
| `server_libraries_set` | `{ tag, ids: string[] \| null }` | – (null includes every library) |
| `jellyfin_check` | `{ url }` | `ServerCheck { url, name, version, quickConnect }` |
| `jellyfin_quick_start` | `{ url }` | `{ code, serverName }` |
| `jellyfin_quick_poll` | – | `{ state: "waiting" } \| { state: "done", server: SavedServer }` |
| `jellyfin_quick_cancel` | – | – |
| `jellyfin_sign_in_password` | `{ url, user, password }` | `SavedServer` |
| `subsonic_check` | `{ url }` | `SubsonicCheck { url, name, version?, openSubsonic, apiKeys }` |
| `subsonic_sign_in` | `{ url, user?, apiKey?, password? }` | `SavedServer` |
| `resume_queues` | – | `ResumeOffer[]` |
| `resume_queue_play` | `{ tag }` | – |
| `resume_queue_save` | – | `boolean` (false when the playing song's server keeps no queue) |

`SavedServer = { tag, kind, name, url?, user?, primary, state?, canTranscode, canDownload }`,
where `state` is `"unknown" | "online" | "offline" | "signInAgain"` for a server
added by address and null for Plex.

A Jellyfin server (10.9 or later) is added by its address. `jellyfin_check`
fails with `invalid` and a message to show when nothing answers there, when
it isn't Jellyfin, or when it's older than 10.9. With Quick Connect on,
`jellyfin_quick_start` gives a code to approve in another Jellyfin app and
`jellyfin_quick_poll` is asked every couple of seconds until it's approved
(an expired code fails with `invalid`). With Quick Connect off, or when the
user prefers it, `jellyfin_sign_in_password` signs in; the password is used
for that one request and never stored or logged. Either way the token goes to
the keychain as `jf.<tag>` and the server's record (no secrets) to
`servers.json`. Without a Plex account, a Jellyfin server is enough to use
the app; its first included library becomes the main one. While Plex is set
up, its library stays the main one and Jellyfin libraries sync next to it.
Jellyfin sees what's playing through its session reports, which are sent so
they don't count a play on their own; each finished play is marked played
once, with the time it happened.

A Navidrome or other Subsonic server is added by its address too.
`subsonic_check` fails with `invalid` when nothing answers there or it doesn't
answer like a Subsonic server, and says whether it takes API keys.
`subsonic_sign_in` takes an API key (preferred; it fails with `invalid` on a
server without API keys and `auth` when the key is refused) or a user name and
password (`auth` when they're wrong). Subsonic checks the sign-in on every
request, so the key or password is kept in the keychain as `os.<tag>`. A
password never goes to the server: each request carries a token made from it
and a new salt instead. Requests are posted as a form where the server allows
it, so the sign-in stays out of addresses; addresses that do carry it (streams,
covers) are never logged, and the log hides `apiKey`, `t`, `s` and `p` values.
What the server can do comes from its OpenSubsonic extension list. The server
can't say what changed since a time, so it's asked every few minutes
(Settings) whether its index or last scan changed, and a sync runs when they
did; edits to music already in the cache arrive with the weekly full sync.
Plays are shown with `playbackReport` where the server has it (told not to
count), and each finished play is scrobbled once with the time it happened.

`ResumeOffer = { tag, serverName, trackCount, currentId, positionMs, changedBy?, changedAt? }`:
a queue another app saved on a server with `resumeQueue`. While "Keep my
queue on the server" is on, the queue is saved on the playing song's server
at each new song and each pause. `resume_queue_play` plays a saved queue
from its song and place. It's not a handoff: the other app keeps playing.

`ServerCaps = { tag?, kind: "plex" | "jellyfin" | "subsonic", name?, primary, caps: Caps }`.
`Caps` is an object with every capability name set to true or false:
`liveUpdates`, `incrementalSync`, `likeDates`, `starRatings`, `playlistMove`,
`playlistPoster`, `playlistSummary`, `serverSmartPlaylists`, `transcode`,
`lyrics`, `syncedLyrics`, `serverRadio`, `sonic`, `playQueueHandoff`,
`resumeQueue`, `remotePlayers`, `friendActivity`, `friendHistory`,
`metadataEdit`, `homeUsers`, `realFilePaths`, `checkFiles`, `loudness`,
`releaseTypes`, `serverHistory`, `backdatedScrobble`, `trackDetails`.
The top level `caps` is what any server in use can do. The UI hides a
feature when the item's server lacks what it needs (an id's tag names its
server; ids without one belong to the primary server), and assumes
everything is there until the first answer. Plex has every capability except
`resumeQueue`, `backdatedScrobble` and `serverRadio`. A Jellyfin server has
`liveUpdates`, `incrementalSync`, `playlistMove`, `lyrics`, `syncedLyrics`,
`serverRadio`, `loudness` and `backdatedScrobble`, plus `transcode` when the
account may convert songs and `friendActivity` and `realFilePaths` for an
administrator. A Subsonic server has `likeDates`, `starRatings`,
`playlistMove` (the playlist is written again in the new order),
`playlistSummary`, `transcode`, `serverRadio`, `friendActivity`,
`resumeQueue` and `backdatedScrobble`; `loudness` and `releaseTypes` when it's
an OpenSubsonic server; `lyrics` and `syncedLyrics` with the `songLyrics`
extension; and `sonic` with `sonicSimilarity`. `home_users` comes back
empty and `now_playing_details` fails with `notFound` for servers without
`homeUsers` and `trackDetails`.

`backends://changed` (no payload) fires when a server is added or removed
and after every `auth://changed`; the UI asks `backend_caps` again. The
servers Platter knows are kept in `servers.json` next to the library cache,
with each server's tag, kind and name and never a token or password.

## Library
| Command | Args | Returns |
| --- | --- | --- |
| `sync_status` | – | `SyncStatus` |
| `sync_now` | `{ full?: bool }` | – |
| `home` | – | `HomeData { shortcuts: HomeItem[], recentlyPlayed: HomeItem[], jumpBackIn: Album[], playlists: Playlist[], recentlyAdded: Album[], topArtists: Artist[] }` |
| `home_shelf` | `{ shelf: "recentlyPlayed"|"jumpBackIn"|"playlists"|"recentlyAdded"|"topArtists", limit?: number }` | `HomeItem[]` (the whole shelf, for Show all; limit defaults to and caps at 200) |
| `home_records` | – | `RecordHome { shown, resume: RecordProgress \| null, newArrivals: Album[], spunThisWeek: RecordSpin[], unfinished: RecordProgress[], notInAYear: RecordLapsed[] }`; `shown` is false (and the rest empty) while `homeRecordsFirst` is off |
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

`home_records` feeds Home in Platter's own looks (Studio, Studio Black and
Sleeve), where it leads with albums. A listening session is the run of plays
started from one album with no pause over two hours between them; only each
album's latest session counts. An album was stopped partway when that
session played at least 2 songs, the album has at least 4, and a song is
left: the next one when the last play finished or was skipped, else the same
one (it was stopped in the middle). `RecordProgress = { album, nextTrackId,
nextTitle, nextNumber, total, playedAt }`: Resume plays the album from
`nextTrackId`, its place `nextNumber` (from 1) of `total`; `playedAt` is the
session's last play in unix ms. `resume` is the newest of these in the last
14 days and `unfinished` the rest from the last 60 days, newest first.
`newArrivals` are albums added in the last 90 days, newest first.
`spunThisWeek` (`RecordSpin = { album, plays }`) are albums with at least 2
counted plays over the last 7 days, most first. `notInAYear` (`RecordLapsed =
{ album, plays, lastPlayed }`) are albums with at least 5 plays (counted
plays or Plex's own count, whichever is higher) whose last play anywhere is
over a year ago, most played first. Each list holds up to 12. Hidden
editions and other servers' copies leave as on Home. The answer is kept
until album plays, counted plays or the library's albums change, or the hour
turns.
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
| `plex_edit_merge_albums` | `{ target, others: string[] }` | `EditChange` |
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
with `CreditRow = { name, artistId?, roles: ("producer"|"writer"|"composer"|"lyricist"|"featured"|"mixer"|"engineer"|"mastering"|"performer"|"arranger"|"conductor")[] }`.

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

`AlbumReleaseStatus = { id, artistId, status: "taggedUnreleased"|"confirmedUnreleased"|"probablyUnreleased"|"plexMissed"|"released", reasons: string[], evidence: Evidence[], mark: "unreleased"|"released"|null, score, mixed }`
with `Evidence = { kind: "tag"|"mark"|"markOverridden"|"unmatched"|"untyped"|"musicbrainz"|"notChecked"|"folder"|"duration", text, weight, decides }`.
`reasons` are the evidence as plain sentences. Tags decide first: a demo
release type or an unreleased `ARCHIVE_VERSION` is `taggedUnreleased`, and
any other version tag is `released`. With no `ARCHIVE_VERSION`, the files'
release status tags (see File tags) rate the album: when more than half of
the disc 1 files with a known status are released it is `released`, whatever
later discs hold, otherwise `taggedUnreleased`. `mixed` is true when its
files hold both released songs and unreleased, leaked or alternate ones, so
an official album with a disc of extras is `released` and `mixed`, and a
build with a few released songs is `taggedUnreleased` and `mixed`. The
user's mark comes next
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
match that has no release type and no version tag on the album or its files
(and that its files' release status tags don't rate unreleased, leaked or
alternate), it searches release groups by artist and title (edition notes
like "(Deluxe)" left out) and takes a sure match only: score 100, the same
folded title and the same artists, and an official release in the group
whose track count is within 2 of the album's. The album's count is its songs
tagged released, else its disc 1 songs, else all of them, so a disc of
extras doesn't hide the match. Found answers are kept 30 days, misses 7
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
and copies on another server), `songStatus?` and `fileRelease?: { status?,
provenance? }` (what the file's tags say, see File tags). A song whose file
has a release status tag takes its own status from it before its album's.

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

### Lineage and Compare
| Command | Args | Returns |
| --- | --- | --- |
| `track_lineage` | `{ id }` (any version of the song) | `Lineage` |
| `compare_prepare` | `{ aId, bId }` | `CompareReady` |
| `compare_switch` | `{ side: "a"|"b", play: bool|null }` | `CompareNow` |
| `compare_loop` | `{ ab: AbLoop|null }` | the loop in the playing version's time |
| `compare_end` | – | – |

`Lineage = { title, artistId, artistTitle, nodes: LineageNode[] }`, where a
node is a `VersionItem` plus `stage` (`demo`, `session`, `leak`, `alternate`,
`unreleased`, `release`, `remaster` or `live`), `when` (yyyymmdd from
`ARCHIVE_DATE`, else the album year, null when neither is known) and
`shareable` (released versions only). Dated versions come oldest first and
undated ones follow in stage order. The stage comes from `ARCHIVE_VERSION`,
then words in the title, album or edition, then the song's status.

`compare_prepare` puts both versions' whole files in the audio cache, one
after the other through the same one-transfer-at-a-time gate playback uses,
and keeps them there until `compare_end`. `compare://progress` reports
`{ side, fraction }` as each file comes in. Downloaded songs count as already
there. It returns `{ aId, bId, approximate, levelMatched, levelAvailable }`:
`approximate` when the lengths differ by more than 5%, so a switch keeps the
same share of the song instead of the same time. While Compare is open,
leveling is track gain when the `compareLevelMatch` setting is on and the
server has loudness data for both, else off. `compare_end` puts the user's
own sound settings back and clears the loop.

`compare_switch` plays one side as a one-song queue at the moment the other
side is at (the start the first time), keeping play or pause unless `play`
says otherwise, and returns `{ side, positionMs, abLoop }`. The loop region
is kept in A's time and carried into B by the same mapping.

Compare leaves the user's listening alone. The first `compare_switch` sets
the whole play state aside: queue, context, position, shuffle, repeat, loop
and play or pause. A song that was playing is reported paused. Until
`compare_end` (or a failed `compare_prepare`) nothing is reported to the
server, written to the listen log, scrobbled, or saved as a resume point or
queue file. Closing puts the play state back exactly, and a song that was
playing carries on as the same play. Now-playing outputs that follow
`player://state` show the compared song meanwhile and go back after.

## Library doctor
| Command | Args | Returns |
| --- | --- | --- |
| `doctor_report` | – | `DoctorReport` |
| `doctor_count` | – | number of findings to fix (the badge) |
| `doctor_ignore` | `{ id, kind, title, ignore: boolean }` | – |
| `doctor_run` | – | – (check every artist again soon) |

`DoctorReport = { on, findings: DoctorFinding[], ignored: DoctorFinding[], count, running, checkedAt?, tagsRead, tagsTotal }`

`DoctorFinding = { id, kind, artistId, artist, albumId, album, thumb?, title, text, evidence: string[], fix, tracks: string[], others: { id, title, thumb? }[], table: { trackId, title, album?, albumArtist?, year? }[], suggest? }`

`DoctorSuggestedTags = { trackId, title, artist, album, newTitle, newArtist, newAlbum?, recordingMbid, score }` (score 0 to 100)

`kind` is one of `duplicate` (the same recording twice on one album),
`cover` (no cover, or one smaller than `doctorCoverPx` on its longest side),
`albumTags` (album name, album artist or year differ between an album's
files), `splitAlbum` (one album shown as two or more: same title and album
artist, no shared songs, the same year, edition, version and release
status; or albums whose files all sit in one folder, a disc folder counting
as its album's, whatever their titles, with no song title in common and the
same edition, version and release status), `albumArtist` (files with no album artist on an album whose songs
credit several artists, or whose name is also under another artist),
`lowQuality` (a lossy copy next to a lossless one of the same recording on
another album, or a low bitrate well under another lossy copy) and
`unmatched` (Plex couldn't match it and the MusicBrainz check found it) and
`soundsLike` (identify by sound matched the file to another recording than
its title, artist or recording id say; files whose names already agree are
left alone). Unreleased songs never match anything, so they are never
duplicates, low-quality copies or `soundsLike`, and unreleased albums are
never `unmatched`.

`fix` names the app's own flow: `cover` (Change cover), `match` (Fix match),
`mergeAlbums` (`plex_edit_merge_albums` after a confirm), `openAlbum` or
`tags` (each song's tags in `table`; they are changed in a tagger) or
`suggestedTags` (the file's tags next to the ones in `suggest`). Low
quality findings also offer a Lidarr request when Lidarr is ready. Nothing
here writes to Plex or to files.

The check runs in the background after a sync, after file tags are read and
after marks change, once the library has been quiet for a short while. It
goes one album artist at a time and skips artists whose albums haven't
changed. Cover sizes are read from the first bytes of each cover through the
same low-priority reads as file tags. `doctor://changed` (`{ count, running }`)
fires when findings or ignores change. Ignored findings are app state in the
sync store (`doctorIgnored`, keyed by finding id) and sync with the library
group.

## Identify by sound
| Command | Args | Returns |
| --- | --- | --- |
| `sound_id_status` | – | `SoundIdStatus` |
| `sound_id_key_set` | `{ key: string | null }` | `SoundIdStatus` |
| `track_sounds_like` | `{ id }` | `SoundsLike` or null |

`SoundIdStatus = { on, appKey, userKey, keyRefused, configured, running, paused, total, done, identified }`

`SoundsLike = { trackId, recordingMbid, title, artist, album?, score, unreleased, inLibrary?: { trackId, albumId } }`

While `identifyBySound` is on (and the app isn't offline), songs are
fingerprinted one at a time: the first two minutes of the file are read in
1 MB ranges at the tag scan's background priority on the transfer gate,
decoded on this computer and turned into a Chromaprint fingerprint
(`rusty-chromaprint`). The fingerprint and the song's length in seconds are
posted to AcoustID's lookup (`meta=recordings releasegroups compress`) with
the application key, nothing else, at most 2.5 requests a second, pausing
after a 429 or 5xx. The best recording scoring 0.8 or more is kept per song
and file; a changed file is done again. A fingerprint whose lookup has to
wait (no key, a refused key, the service busy) is kept so the file isn't
read twice.

The key is the user's own from `sound_id_key_set` (kept in the keychain as
`acoustid-key`), else the one the app was built with
(`PLATTER_ACOUSTID_KEY`). With neither, `configured` is false and nothing
runs. `keyRefused` is set when AcoustID answers with error code 4 and clears
when a key is saved.

The recording feeds the matcher's fingerprint tier (`song_index.sound_mbid`,
between Plex's track id and the names) and the library doctor's
`soundsLike` finding. It never goes against two different recording ids in
the files' tags, and unreleased songs never match anything, so for them
`track_sounds_like` is only a hint. `inLibrary` is a released copy of that
recording in the library. `soundid://changed` fires as songs are done.

## Automix and song analysis
| Command | Args | Returns |
| --- | --- | --- |
| `analysis_status` | – | `{ on, running, total, done }` |
| `automix_mixed_get` | – | `{ [playlistId]: MixedPlaylist }` |
| `automix_mix_set` | `{ id, on }` | `{ [playlistId]: MixedPlaylist }` |
| `automix_style_set` | `{ id, fromId, toId, style }` | `{ [playlistId]: MixedPlaylist }` |
| `automix_view` | `{ playlistId?, trackIds }` | `AutomixView` |

`MixedPlaylist = { styles: { ["<fromId>><toId>"]: MixStyle } }`; a playlist
is mixed when it's in the map. `MixStyle = "auto" | "none" | "blend" | "lift" | "drop" | "echo"`;
`"auto"` removes a pick. Kept in the sync store (`mixedPlaylists`, keyed by
playlist id) with the library group.

`AutomixView = { songs: [{ id, bpm?, key? }], gaps: [{ fromId, toId, reason?, style, beatmatched }] }`
for songs in the order shown. `bpm` is rounded and only given for a steady
beat; `reason` says why a gap can't be mixed. On a mixed playlist, songs not
analysed yet are analysed first.

While `automix` is on, each song is analysed once per file on this computer:
the whole file is read (from the pre-cache when it's there, otherwise at the
lowest priority on the transfer gate), decoded to mono at 11 kHz, and
measured with a short-time Fourier transform: an onset curve gives the tempo
(autocorrelation) and the beats (dynamic programming), straight lines fitted
through the beats of the first and last minute give beat grids, bars start
where the low end and the harmony change most, the key comes from
Krumhansl-Kessler key profiles, plus energy, loudness every 2 s, where the
music starts and stops, and a 20-number feature vector. Work runs on one
thread at idle priority, one song at a time with a pause between songs, and
waits while the computer is busy (the transcription busy check). Songs over
20 minutes aren't analysed. `analysis://changed` fires with the ids of songs
analysed.

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
| `miniplayer_go` | `{ kind: "album"\|"artist", id }` | – |

Event `miniplayer://changed` → `{ open }` when the window opens or closes
(from either window, or because the main window closed).

`miniplayer_go` is the queue menu's Go to album: it brings the main window
forward and sends it event `miniplayer://go` → `{ kind, id }`, and the main
window opens that page. An empty `id` fails with `invalid`.

The miniplayer is a second window, label `miniplayer`, with no frame and its
own capability (`capabilities/miniplayer.json`: dragging and resizing only).
It loads the same page as the main window, which sees the label and shows
just the miniplayer, fed by the same `player://` events. It opens at 340 x 365
in the bottom-right corner of the main window's screen, or where it was last
when that place is still on a screen (`miniplayerBounds`, saved a moment
after it stops moving and when it closes). It is always on top while
`miniplayerOnTop` is on; changing that setting applies at once. A change to
any `miniplayer*` setting is sent to both windows as `settings://changed`.
Turning `miniplayerQueue` on while the window is shorter than 500 makes it
500 tall, moved up when that would run off the bottom of its screen. The
window can be resized from 240 x 260 up to 1000 x 1000, and it has the
system's file drop turned off so the queue's rows can be dragged to reorder.
When the window can't be made, `miniplayer_set` fails with `internal` ("The
miniplayer couldn't open.").

## Main window
| Command | Args | Returns |
| --- | --- | --- |
| `window_ready` | `{ background: string \| null }` | – |

The main window is made hidden and shown once its page has drawn. The UI
calls `window_ready` after the shell is on the page with the look applied
(two animation frames, or 150 ms if frames don't run), passing the look's
background as `#rrggbb`. If it never comes the core shows the window after
3 seconds anyway. Launched at login with close to tray on, the window stays
in the tray; otherwise it opens minimised. Calls from other windows are
ignored.

The core keeps `window.json` in the app data folder (shared by all Home
users on the computer): the window's place, size and maximised state, saved
when it closes or the app quits, and the look's background, which colours
the window before the page draws on the next launch. A place that is no
longer on any screen is dropped and the window opens centred.

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
| `automation_revoke` | – | `DeckStatus` |
| `deck_pair_prompt` | – | `string \| null` |
| `deck_pair_answer` | `{ id: string, allow: bool }` | `bool` |
| `deck_plugin_status` | – | `DeckPluginStatus` |
| `deck_plugin_install` | – | – |
| `deck_app_download` | – | – |

`DeckStatus` is the port's state as in `FeedStatus` (`off`, `on` with
`port`, `portInUse`, `failed`) plus `code` (the six-digit pairing code, only
while it's on) and `paired`. Event `deck://status` carries it whenever it
changes, including after a pairing. `deck_revoke` forgets the paired Stream
Deck and makes a new code. It's turned on with `settings_set`
(`allowStreamDeck`). Automation has its own pairing on the same API:
`automationCode` (only while `allowAutomation` is on) and
`automationPaired`, and `automation_revoke` forgets it. The port is open
while either setting is on. See [AUTOMATION.md](AUTOMATION.md).

The Stream Deck plugin can pair without the code. When it asks
(`POST /v1/pair/request`, below) the window comes up and event
`deck://pair-request` carries `{ id }`; the UI shows "Allow Stream Deck to
control Platter?" and answers with `deck_pair_answer` (`allow` false for
Don't allow, and for closing the prompt). `deck://pair-request` with
`{ id: null }` closes it, and `deck_pair_prompt` gives the open prompt's id
when the window loads. Nothing else can allow it.

`deck_plugin_status` says whether the Stream Deck app is on this computer
and the bundled plugin installed: `DeckPluginStatus` is `{ state: "noApp" }`,
`{ state: "notBundled" }` (a build without the plugin), `{ state: "install",
version }`, `{ state: "installed", version, installed }` (same version or
newer) or `{ state: "update", version, installed }`. Windows looks for
`StreamDeck.exe` under Program Files and `%APPDATA%\Elgato\StreamDeck`;
macOS for `/Applications/Elgato Stream Deck.app` and
`~/Library/Application Support/com.elgato.StreamDeck`. The installed version
is read from the plugin's `manifest.json` in that folder's `Plugins`.
`deck_plugin_install` opens the bundled
`streamdeck/app.platter.streamdeck.streamDeckPlugin` resource with the OS,
so the Stream Deck app installs it (the UI turns on `allowStreamDeck`
first). `deck_app_download` opens Elgato's download page.

Request album and Listen along from a Stream Deck bring the window up and
send `deck://action` to the UI, which shows the usual confirmation:
`{ kind: "requestAlbum", input: LidarrRequestInput }` opens the Lidarr
request dialog, and `{ kind: "listenAlong" }` starts a session (or shows the
open invite). For automation, `{ kind: "open", route }` opens a page and
`{ kind: "notice", message }` says why a `platter://` command link didn't run.

The control API itself is plain HTTP with JSON on `127.0.0.1:32541`, for the
Stream Deck plugin in `streamdeck/` and other local programs. It binds the
loopback address only, answers only loopback peers whose `Host` is
`127.0.0.1:32541` or `localhost:32541`, refuses any request with an `Origin`
header or a `Sec-Fetch-Site` other than `none` (403), and allows 20 requests
a second (429 past that). Bodies are at most 4 KB.

- `GET /v1/hello` → `{ app: "Platter", api: 1, paired }`; `paired` says
  whether the bearer token sent is the paired one.
- `POST /v1/pair` with `{ code }` → `{ token }`, or `{ code, client:
  "automation" }` for the automation pairing (403 while its setting is off,
  as is a Stream Deck pairing while Allow Stream Deck is off). One try every two seconds;
  a wrong code is 401 and five in a row make a new code. A success also
  makes a new code and replaces any earlier token. Only the token's SHA-256
  is kept, in the keychain.
- `POST /v1/pair/request` (Stream Deck only, 403 while Allow Stream Deck is
  off) → `{ request }`, a random id, and the window asks the person. One
  prompt at a time (409 while one is open), at least five seconds between
  two asks and after an answer (429), and it shares the pairing limit above.
  A prompt left for two minutes goes away.
- `POST /v1/pair/status` with `{ request }` → `{ state: "waiting" }`,
  `{ state: "allowed", token }` (once; the token replaces any earlier one,
  as with the code), `{ state: "denied" }` or `{ state: "gone" }`
  (expired, collected or unknown).
- Everything else needs `Authorization: Bearer <token>` (401 without it).
- `GET /v1/state?cover=<coverKey>&size=<px>` → `{ api, playing, private,
  title, artist, album, trackId, liked, volume, muted, coverKey, cover,
  canRequest, positionMs, durationMs, shuffle, smartShuffle, repeat,
  crossfade, lyrics, miniplayer, window, device, stars, canRate, playlists,
  next, previous, context, sleep }`.
  `volume` is 0 to 100. `cover` is a `data:image/...;base64,` image, 144 px
  unless `size` asks for more (up to 288, for XL and high-DPI keys), left out
  when `cover` in the query already names it (or is `none`). A private
  session leaves out the song, the cover, `next` and `previous`. `repeat`
  is `off`, `all` or `one`. `crossfade` is on unless Settings has it off.
  `lyrics` is the expanded view showing lyrics (the window reports it with
  `deck_ui_report`), `miniplayer` the miniplayer being open and `window` the
  main window showing. `device` is the Listen on device playing now (null
  here). `stars` is the song's rating, 0 to 5, and `canRate` whether its
  server has star ratings (Plex). `playlists` lists the ids of the playlists
  holding the song. `next` is `{ title, artist, coverKey }` for the song
  after this one and `previous` the same for the one before, `context`
  `{ kind, id }` for what's playing from (as in `play_context`), and `sleep`
  `{ remainingMs, endOfSong }` while the sleep timer runs.
- `GET /v1/cover?key=<coverKey>&size=<px>` → `{ key, cover }` for any cover
  key the API has handed out (the state, `next`, a list); 404 for one it
  doesn't know.
- `GET /v1/library?kind=playlists|albums|artists|mixes` → `{ kind, items: [{
  id, title, subtitle, coverKey }] }`, at most 500: playlists by name, albums
  newest added first, artists most played first, mixes as Home has them.
- `GET /v1/queue` → `{ items: [{ itemId, title, artist, coverKey }] }`, the
  user queue then what follows, at most 30 (none in a private session).
- `GET /v1/devices` → `{ items: [{ id, name, active, needsWindow }] }`, the
  Listen on list with this computer first (id `local`). `needsWindow` marks a
  device that needs a password or can't be played on.
- `POST /v1/command` with `{ command }`: `playPause`, `next`, `previous`,
  `like` (toggles), `volumeSet` and `volumeStep` (with `value` in percent),
  `requestAlbum`, `listenAlong`, `play`, `pause`, `playPlaylist` (with
  `playlist`, an id or a name, and optional `shuffle`), `search` (with
  `query`) and `open` (with `page`), and for the Stream Deck keys:
  - `seekBy` with `value` in seconds (negative goes back);
  - `shuffle` (toggles; with `smart: true` toggles smart shuffle), `repeat`
    (off, all, one, off), `mute` (toggles, keeping the volume),
    `crossfade` (Settings' crossfade between off and smart) and
    `privateSession` (toggles the setting);
  - `addToPlaylist` with `playlist` (409 when the song is in it already), or
    without one to bring the window up with the playlist picker
    (`deck://action` `{ kind: "addToPlaylist", trackIds }`);
    `removeFromPlaylist` with `playlist`;
  - `rate` with `stars` 1 to 5 (the rating the song has clears it; only on a
    server with star ratings);
  - `playItem` with `kind` (`playlist`, `album`, `artist`, `mix`), `id`,
    optional `shuffle`, `queue` (add it to the end of the queue instead) and
    `start` (seconds into its first song);
  - `playLink` with `link` (a `platter://` sharing link; a song plays its
    album from that song, at its time), `playSearch` with `query` (the top 50
    songs), `radio` (radio from the playing song) and `playShortcut` with
    `name` `daylist`, `discover` or `liked`;
  - `queueJump` with `itemId`;
  - `listenOn` with `device` from `/v1/devices`, or without one (or for a
    device that needs the window) to bring the window up with the Listen on
    picker (`deck://action` `{ kind: "listenOn", device? }`);
  - `lyrics` (`deck://action` `{ kind: "lyrics", open }`; the window comes up
    to open them), `miniplayer`, `window` (shows or hides the main window),
    `nowPlaying` and `queue` (bring the window up on the expanded view or the
    queue: `{ kind: "nowPlaying" }`, `{ kind: "queue" }`);
  - `sleep` with `minutes` (up to a day) or `endOfSong: true`: one timer for
    every client, replaced by the next; minutes pause playback, end of song
    stops the next song as it starts. `sleepCancel` stops it.
  - `bookmark` (the playing song at this moment, as in Bookmarks).

  → `{ ok: true }`, or 409 with `{ error }` saying why (nothing playing,
  Lidarr not set up, no album to request, no such playlist or page, already
  in the playlist). A Stream Deck token works while Allow Stream Deck is on,
  an automation token while Allow automation is on.

| Command | Args | Returns |
| --- | --- | --- |
| `deck_ui_report` | `{ report: { lyrics } }` | – |

The window calls `deck_ui_report` when the expanded view opens or closes
lyrics, for the Lyrics key.

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
artist and title. The file's own album, album artist and year are kept too
(Vorbis `ALBUM`, `ALBUMARTIST` or `ALBUM ARTIST`, `DATE` or `YEAR`; ID3
`TALB`, `TPE2`, `TDRC` or `TYER`; MP4 `©alb`, `aART`, `©day`), for the
library doctor. For each file's release status the core keeps `STATUS`,
`VERSION` and MusicBrainz Picard's release status and type (Vorbis
`RELEASESTATUS` and `RELEASETYPE`, ID3 `TXXX` and MP4 `----` items named
`MusicBrainz Album Status` and `MusicBrainz Album Type`), plus the custom
status tag when `releaseStatusTag` names one (Vorbis comment, ID3 `TXXX` or
MP4 `----` item of that name). Each cached row carries the cache version it
was read with and the custom tag name it was read for; when a new version
keeps more, or the tag name changes, older rows are read again after every
unread file, and they keep working until then.

Each file gets one release status, `released`, `unreleased`, `leaked`,
`alternate` or unknown, from the first of these with a known value:
`ARCHIVE_VERSION` (leak is leaked, alternate is alternate, the other
unreleased kinds are unreleased, any other version is released), then with
`releaseStatusTag` on Auto `STATUS`, Picard's release status (official,
promotion and withdrawn are released, bootleg and cancelled unreleased,
pseudo-release unknown) and a Picard release type with a demo part
(alternate), or else the custom tag. Values are compared in any case, with
`_` and `-` as spaces and, failing that, without anything in brackets
("Official release (web rip)" is released). `releaseStatusValues` come
first, then the built-in values (released, official, retail, promo;
unreleased, unofficial, bootleg, scrapped, vault; leak, leaked; alt,
alternate, demo, og, og file, reference, snippet, session and a few more).
A value none of them knows stays unknown and never counts as released. The
provenance note is `VERSION` as written, else `ARCHIVE_SOURCE`. Changing
`releaseStatusTag` or `releaseStatusValues` works every file's status out
again from the tags already read (then `tags://changed`), and a new tag
name reads the files again in the background.

Reads go one at a time through the player's transfer gate at the lowest
priority, below pre-caching, and only while playback has nothing left to
download. Any playback transfer that wants the gate cuts the read, which is
sent again later. A server that ignores Range gets a whole-file read only for
files under 2 MB; bigger ones are skipped. Results are cached in SQLite by
part key, which carries the part's update stamp, so a changed file is read
again and its old row dropped. A file that can't be read (404, bad data) is
marked and left until it changes. Network or server trouble, or a relay-only
connection, pauses the scan (`paused`) and it tries again a minute later.

`tracks` maps track ids to `{ explicit?, version?, quality?, edit?, release?,
provenance? }` (only songs with something to show): `release` is the file's
release status and `provenance` its note. `albums` maps album ids to `{
explicit?, version?, group?, edition?, release?, mixed? }`: explicit when
any song is, the most common version, edition and group, the album's status
by its files' release status (disc 1 decides, see `AlbumReleaseStatus`) and
whether its files mix released and other material. A song row's version
badge shows `version`, else `release`. `groups` maps a group key to its editions
`{ albumId, title, label, year }[]`, two or more, the face first. The face is
the edition labelled Standard, else a released one, else the earliest.

`ARCHIVE_VERSION` joins the unreleased check for sharing and Discord: any of
unreleased, leak, leaked, demo, snippet, session, build or alternate counts.
Without one, the file's release status does (unreleased, leaked and
alternate count), and for an album the status its files give it. Every
check that asks whether music is unreleased (sharing, Discord, scrobbling,
mixes, smart playlists, Listen along, lyrics, the archive views and the
song index) reads it this way. With
`groupEditions` on, `albums`, `artist_page`, `home` and `home_shelf` leave out
editions behind their face, and `album_page` leaves its own editions out of
More by. Play contexts and album pages follow `explicitContent` and
`showFanEdits` (see Settings). A context that loses every song fails with
`invalid`.

## Tag editor
| Command | Args | Returns |
| --- | --- | --- |
| `tagedit_pick_music_folder` | – | `MusicFolderCheck \| null` (null when the dialog is cancelled) |
| `tagedit_set_music_folder` | `{ path }` | `MusicFolderCheck` |
| `tagedit_clear_music_folder` | – | – |
| `tagedit_music_folder_status` | – | `MusicFolderCheck \| null` (null when not set) |
| `tagedit_open` | `{ albumId \| null, trackIds: string[] }` | `TagEditorData` |
| `tagedit_save` | `{ request: { label, files: { trackId, stamp, edits }[] } }` | `TagSaveResult` |
| `tagedit_undo` | `{ id }` | `TagSaveResult` |
| `tagedit_history` | – | `{ id, at, label, files }[]`, newest first, at most 20 |
| `tagedit_check` | `{ scope: { kind: "album" \| "artist", id } \| { kind: "library" } }` | `TagCheckReport` |
| `tagedit_check_fine` | `{ id, fine }` | – |
| `tagedit_fix_preview` | `{ kind: "discTags" \| "releaseStatus", albumId }` | `TagFixPreview` |
| `tagedit_isrc_releases` | `{ albumId }` | `{ candidates: IsrcRelease[], wanted, message }` |
| `tagedit_isrc_preview` | `{ albumId, release: IsrcRelease }` | `TagFixPreview` |
| `tagedit_explicit_preview` | `{ albumIds: string[], explicit, artistId \| null, era \| null }` | `TagFixPreview` |
| `tagedit_library_settings` | – | `{ id, title, agent, items: { label, ok, help }[], ok }[]` |

The only commands that write to the user's files. They work on the music
folder (`musicFolder`): the folder this computer sees the server's music in.
Picking it (or typing a path) looks for 5 random songs: Plex's file path for
each one is matched to the folder by the longest end of the path found under
it, and the file must have the size Plex has. `MusicFolderCheck` is `{ ok,
found, checked, message, mapping: { server, local } \| null, probes }`, with
`message` like "Connected, 5 of 5 files found" or what's wrong. The folder is
saved (as `musicFolder` and `musicFolderServer`) when at least one file was
found.

`TagEditorData` is `{ title, files, scheme, statusField, statusChoices,
customKey, writable, reason, needsMusicFolder }`. Each file is `{ trackId,
albumId, serverPath, localPath, fileName, format, id3Version, values, keys,
stamp, error, durationMs }`; `values` maps field names (`albumArtist`,
`album`, `year`, `genre`, `releaseType`, `edition`, `group`, `era`,
`quality`, `explicit`, `trackNumber`, `trackTotal`, `discNumber`,
`discTotal`, `title`, `artist`, `archiveVersion`, `status`, `picardStatus`,
`customStatus`, `versionNote`, `archiveDate`, `source`, `flags`, `isrc`) to
what the file holds. Without a music folder the values come from Plex and the
tag cache, `stamp` is null and `needsMusicFolder` is true: details are view
only. `scheme` (`archive`, `plain`, `picard` or `custom`) is where this
album's release status is written, and `statusField` the field for it: the
tag most of its files already carry, else the custom tag from Settings, else
`ARCHIVE_VERSION`.

`tagedit_save` sends only changed values, each with the `stamp` the file had
when it was opened; `""` removes a tag. Values are checked first (an
`invalid` error for a bad ISRC, track number or date, with nothing written).
Then every file is read again: when any changed since it was opened, is gone,
or its folder had a file changed by another program in the last minute,
nothing is written and `problems` says why (`changed`, `gone`, `busy`). Each
file is then written through a temp copy in its own folder and replaced; a
failure leaves it as it was (`readOnly`, `locked`, `failed`). `TagSaveResult`
is `{ saved, unchanged, problems, message, undoId, rescanned, rescanError }`,
`message` like "Saved 12 files". With `rescanAfterTagEdit` on, Plex is asked
to rescan each folder written to (`rescanned`, server paths). The tag cache
takes the new values at once and `tags://changed` is sent.

`tagedit_undo` puts back what an edit replaced on each file that is still as
the edit left it; files changed since are kept as they are and listed in
`problems`. An edit is undone once.

`TagCheckReport` is `{ findings, styles, albums, files, ignored, fromFiles }`.
A finding is `{ id, rule, severity: "error" \| "warning" \| "info", albumId,
albumTitle, trackIds, what, why, fix }`; `fix` is `{ albumId, trackIds, field
}` (opens the editor there), or null without a music folder. Findings marked
fine with `tagedit_check_fine` are left out and counted in `ignored`.
`styles` gives each album's learned style ("This album's style: ...").

`TagFixPreview` is `{ kind, label, changes, kept, pickEach, message }`. Each
change is `{ trackId, fileName, stamp, field, before, after, note }`; the UI
saves the picked ones with `tagedit_save` and `label`, so they are logged and
can be undone. `pickEach` (release status) means none start ticked. `kept`
lists files left as they are with the reason. `IsrcRelease` is `{ source:
"musicbrainz" \| "deezer", id, title, artist, date, country, tracks }`; the
lookup errors with `network` ("MusicBrainz and Deezer couldn't be reached.
Nothing was changed; try again later.") when neither answers.

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
| `lyrics_candidates` | `{ trackId }` | `LyricsCandidates` |
| `lyrics_check_status` | `{ trackId }` | `CheckStatus` or null (see Checking lyrics against the song) |
| `lyrics_choose` | `{ trackId, pick }` | – |
| `lyrics_offset` | `{ trackId, offsetMs }` | – |
| `lyrics_choices_count` | – | `number` |
| `lyrics_choices_clear` | – | `number` |

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

### Checking lyrics against the song

Every set of lyrics found for a song is a candidate, in this order: the
user's own when they prefer them, the server's, the user's own, LRCLIB's
exact lookup, every LRCLIB search hit with the same artist and title
(closest length first), then a transcription. When `lyricsCheckAudio` is on
and a speech model is installed, Platter transcribes a 30 second window of
the song on this computer (starting just before the first sung line, reading
it at the lowest priority, never ahead of playback's transfer, and with the
background thread budget and busy pause of Lyrics transcription) and compares the words
heard with each candidate in turn until one matches. Lyrics the user wrote
and transcriptions aren't checked. Results are kept per song, candidate and
text, so a check runs once. The audio never leaves the computer.

A candidate that matches is shown with `check: "checked"`. When its timing is
off by a steady amount, or drifts (measured again near the end), and
`lyricsFixTiming` is on, the lines come back moved (`autoOffsetMs`,
`retimed`). A candidate is only skipped when the check is sure it doesn't
match: the first window and at least one more from another part of the song
(the middle, later or earlier; windows that are quiet or have too few words
are skipped) each heard plenty of words and scored well under the line, and
no window matched or came close. One window alone, too few words, or
windows that disagree (a song with a beat switch) can't tell: the lyrics
show with `check: "notChecked"`. While a check runs, lyrics from the server
or LRCLIB's exact lookup show with `check: "checking"`; ones found only by
title wait (`notice: "checking"`). When the check is sure nothing matched,
`notice: "noneMatched"` (and a transcription is queued when transcription
is on). `lyrics://checked` (`{ trackId }`) says to ask again.

A transcription made on this computer only fills in: it shows when nothing
found can be (none found, every one surely wrong, or held back as another
version's), and never ahead of lyrics found for the song unless the user
picks it ("Transcribe this song" picks the one it asks for). Updating from a
build that judged lyrics on one window drops those no-match results, so the
songs are checked again and the lyrics found show meanwhile; transcriptions
they led to are kept and listed by `lyrics_candidates`.

The check uses the smallest installed model and two threads. With the small
model it has a short-lived helper process of its own (see Lyrics
transcription); with a bigger model, or while a graphics card is in use,
it uses transcription's helper, ahead of background songs (so only one model
sits on the card), and takes the newest song first (the
one playing); older requests wait, and beyond three the oldest are dropped.
`lyrics://check-status` → `CheckStatus = { trackId, state, reason? }` says where
a song's check is, and `lyrics_check_status { trackId }` returns the latest one
(or null). `state` is `queued`, `waiting` (reason `busy`: the computer is busy),
`reading` (the song), `listening` (the model on a window), `done`, or `failed`
with reason `noModel`, `unreadable`, `later` (couldn't read or listen now, or
dropped from the queue) or `stuck` (no progress for about two minutes). The
lyrics view shows these in words ("Waiting: your computer is busy", "Can't
check without the speech model", "Couldn't check yet" with Try again, which
asks for the lyrics again and so for a new check).

Without the check, lyrics show with `check: "notChecked"` and stricter rules:
LRCLIB timing is kept only when its length is within 2 s of the song's, or
within 5 s on the same album (edition notes ignored). A song that is
unreleased, an alternate version, live, or has a version tag doesn't take
lyrics found for another release unless `lyricsReleasedForVersions` is on,
the user picks them, or the check matched them: `notice: "versionHeld"` with
the held candidate in `pick`. `disagree` is set when other lyrics found say
something quite different.

`lyrics_candidates` lists every candidate (`key`, `origin`, `source`,
`album`, `durationMs`, `synced`, `check`, `otherVersion`, a short
`preview`, `shown`) with the user's `pick` and `offsetMs`, and `canCheck`.
`lyrics_choose` sets the song's pick: a candidate key, `"none"` (`notice:
"chosenNone"`), or `null` for automatic. A new pick starts with no offset.
`lyrics_offset` moves synced lines by `offsetMs` (positive is later, rounded
to 0.1 s, at most a minute either way). Picks and offsets win over anything
automatic and travel in the sync file (`lyricsChoices`); `lyrics://choices`
says some arrived from another device.

| Command | Args | Returns |
| --- | --- | --- |
| `lyrics_pronounce` | `{ lines: string[] }` | `(string \| null)[]` |
| `lyrics_translate` | `{ lines: string[] }` | `string[]` |

`lyrics_pronounce` gives a romanised line for each lyrics line, or `null`
where there's nothing to romanise. It's made on this computer: Japanese with
the kakasi crate (KAKASI's dictionary, Hepburn), Chinese with the pinyin
crate (tone marks, one reading per character), and Korean, Hindi
(Devanagari) and Cyrillic from tables in `lyrics/pronounce.rs`. Kanji read
as Chinese unless the song has kana somewhere. The UI asks only while
`showPronunciation` is on.

`lyrics_translate` sends the non-blank lines to the service in
`translationService` (`POST {service}/translate` with `q`, `source: "auto"`,
`target` from `translateTo` and `format: "text"`, the LibreTranslate API)
and returns one line per line, blank lines left blank. With no service set
it fails with "Set a translation service in Settings first." and sends
nothing. There's no default service.

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
| `transcribe_pause` | `{ paused: bool }` | – |
| `transcribe_redo` | – | songs queued (number) |
| `transcribe_found_nothing` | `{ trackId }` | bool |

Events: `transcribe://status` → `TranscribeStatus` on every change (progress
at most four times a second), and `transcribe://done` → `{ trackId, found }`
when a song is finished (`found` false when no words came out).

Opt-in with `transcribeLyrics`. Speech recognition runs on this computer with
whisper.cpp; nothing about the song is sent anywhere. The only network use is
the model download, from the whisper.cpp model repository on Hugging Face,
pinned to one commit: `small` is `ggml-small.en-q5_1.bin` (190,098,681 bytes)
and `medium` is `ggml-medium.en-q5_0.bin` (539,225,533 bytes), both
English-only, and `turbo` is `ggml-large-v3-turbo-q5_0.bin` (574,041,195
bytes), multilingual and made for the graphics card. The file goes to `<app data>/models/` as a `.part` file, is
checked against its SHA-256, and is renamed into place only when it matches.
Free space is checked first (the model plus 256 MB). Models never ship in the
installer.

`TranscribeStatus = { enabled, available, unavailableReason?, model,
modelPicked, gpu?, gpuFound?, models, current?, queued, upNext?, batch,
secsPerSong?, paused?, error?, userPaused, redo }`. `available` is false in a build
without the recognizer, or on an x86 processor without AVX2. `model` is the
model used: `transcribeModel` when one is picked (`modelPicked`), else
`turbo` while a graphics card is in use and `small` without. `gpu` names the
graphics card in use, `gpuFound` a usable one whether used or not. `models`
lists `{ kind, sizeBytes, installed, downloading, doneBytes, secsPerSong? }`
for each model, so the size shows before downloading; `secsPerSong` is how
long a 3.5 minute song took lately with that model on the device used now
(the graphics card or the processor), from songs of 30 seconds or more
transcribed here (kept in `<app data>/models/speeds.json`). `current = { trackId, stage, percent,
background, quietSecs?, title?, waitingForPlayback?, model }`, stage one of `waitingForModel`, `reading`,
`decoding`, `loading` (the helper is loading the model), `transcribing`
(percent counts the last; the speech library reports it once per 30 s
stretch of the song). `quietSecs` appears once the stage and percent haven't
moved for two minutes, and the lyrics view says so ("No progress for 2
minutes…") instead of looking stuck. `upNext` is the song that goes next. `batch = { done, total }` counts
songs since the queue was last empty. `secsPerSong` is about how long each
song of a run of more than one takes, the rest between songs included, once
one is done. `paused` is `battery`, `offline` or `busy` (songs nobody is
waiting on only; `battery` for the background batch only). `error = { kind,
message, trackId? }` with kind `download`, `checksum`, `disk` or
`transcribe`; it clears when something new starts. `title` is the song's
title. `waitingForPlayback` is true while reading the song waits because the
playing or next song is downloading. `model` is the model the song uses: the
chosen one, or while that one downloads, an installed one (the bigger first),
so the batch keeps going instead of waiting. `redo` lists `{ model, count }`
for songs done this session with a model other than the chosen one;
`transcribe_redo` queues them again with the chosen model once it's installed
(it fails with `invalid` before). Results from the older model stay until the
new ones replace them.

`transcribe_pause` with `paused: true` pauses the batch (`userPaused`): the
song in progress stops at the speech library's next check and goes back in
the queue, and the queue and `batch` stay. Songs asked for one at a time
still run. A paused batch is kept in the database and comes back paused after
a restart. `paused: false` carries on. `transcribe_cancel` without a track
clears it.

Before transcribing any song, when `lyricsOfficialFirst` is on and the check
against the recording can run, the core gathers the song's synced lyrics
(the server's, files next to the song, LRCLIB's lookup and search hits) and
checks those not judged yet. If one matches, the song isn't transcribed: the
match is what the lyrics view shows. A song whose lyrics choice is the
transcription is always transcribed. Each week, at idle, transcribed songs
are looked at again the same way; a match then shows in place of the
transcription, which stays under "Wrong lyrics?". A transcription is also
given to the check, which measures its timing against a window of the song,
and a steady offset is corrected like any checked lyrics.

Word times come from whisper.cpp's attention alignment, which marks where
each token ends; a word starts about one token's length earlier, never
before the word before it. Transcriptions stored with the first timing
(words at their first token's end, about a quarter second late) are moved
into place once, at idle, from the stored word times without hearing the
songs again; the timing each one was made with is kept in
`lyrics_auto_timing`. Words the check heard with the old timing, and the
offsets measured from them, were dropped in the same update.

`transcribe_track` puts the song first and runs it now, battery or busy or
not. A background song in progress is paused, not run beside it: it stops at
the speech library's next abort check (the end of the step it's on), goes
back at the front of the background songs with its decoded audio kept, and
goes on once the asked-for song is done. The asked-for song runs in the same
helper with the model already loaded. A background song waiting for a pause
to end gives way the same way, and so does one in progress when a lyrics
check needs the medium model. While it waits its turn the lyrics view says
"Pausing the background song first…" with Cancel. `transcribe_cancel` on the
song in progress asks the helper to stop; one that hasn't within about a
second is ended (it would have ended a minute later with nothing to do). A
song still queued is just taken out; the one running is left alone.
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
mixed to mono and resampled to 16 kHz. The batch waits while the computer is
on battery. A slow connection never pauses anything: a song playback just
fetched is read from disk, and before reading a song from the server it waits
for playback's own download to end.

The speech model runs in helper processes: one for transcription, and for
the lyrics check only with the small model on the processor a second that
ends 10 s after its last run. The bigger models are only ever loaded by the
transcription helper, so they are never in memory twice. The transcription
helper uses the graphics card while one is in use (it is started with
`PLATTER_SPEECH_GPU=1`); a crash while on the card moves transcription to the
processor for the rest of the session. Each is the same Platter binary started
with `--speech-worker`, which exits before any of the app starts (no window,
tray, single-instance lock or link handling). The app sends it a request
(model file, model kind, threads, language) and the song's 16 kHz samples on
its stdin; it answers on stdout with a hello, progress (`loading` the model,
then the percent), then the segments, a failure or cancelled. Each frame is a marker byte, a kind byte (`J` JSON, `S`
samples), a 32-bit little-endian length and the body. A cancel is a message of
its own. The helper starts when work arrives, stays while there is work, and
ends a minute after the last run, which frees the model's memory; it also ends
when its stdin closes (the app quit). On Windows it starts without a window at
below normal priority (idle priority when `transcribeProcessor` is `light`),
with power throttling (EcoQoS) on; on macOS it runs as a background process,
and the model runs on the GPU with Metal; on Linux it's niced. The player's
own process is never lowered. A helper that crashes or answers nonsense fails
that song ("The speech model stopped unexpectedly on this song.") and the
queue goes on with a new one; after three crashes in a session albums, the
batch and the lyrics check stop until Platter restarts (the error says so),
and songs asked for still run.

Threads follow `transcribeProcessor`: `light` gives albums and the batch two
and the one song asked for four, and a background song that starts while a
lyrics check runs gets one (a song already running keeps its count for the
check's few seconds); `balanced` four for both;
`fast` half the logical cores, at most eight. Never more than half the
logical cores. A lyrics check gets two. The count is whisper.cpp's
`n_threads`, which includes the helper's thread that runs it; the helper has
one more thread of its own that only waits on its stdin for a cancel, so the
operating system shows one thread more than the count. With `transcribeFasterAway`, while there has been no keyboard
or mouse input for 5 minutes and the computer isn't on battery, background
work gets at least the `balanced` count; when input comes back, a song
running with the extra threads starts again with the usual count. Background
songs rest 5 s between each other.

With `transcribePauseBusy`, background work (albums, the batch, the lyrics
check) waits (`paused: busy`) while other programs use more than half the
processor averaged over about 10 s (Platter and its helper left out), while a
fullscreen Direct3D game or presentation mode is in front (Windows), or while
macOS reports a thermal state of Fair or worse; it goes on after 30 s without
any of them. The computer is read every 3 s, and only while background speech
work is queued; nothing is read while the queue is empty.

On Windows and Linux the model runs on the graphics card with Vulkan (any
vendor), and on macOS with Metal, while `transcribeGpu` is `auto` and a usable card is found. The app
never touches the card itself: once transcription is on (or a lyrics check
runs), it starts the same binary with `--speech-probe`, which lists the cards
the speech library sees as one JSON line and exits; a probe that fails,
crashes or takes more than 20 s means no card. A discrete card is picked over
an integrated one, then the one with the most memory; cards reporting less
than 1 GB are left out. The helper is started with `PLATTER_SPEECH_GPU=1`
when it may use the card, logs one line saying which card or the processor
it loaded the model on, and loads it on the processor when loading on the
card fails. A helper that crashes while on the card moves transcription to the
processor until Platter restarts. Vulkan comes with the graphics driver: the
binary doesn't link the Vulkan loader, the helper opens it (`vulkan-1.dll`
from System32, or `libvulkan.so.1`) only when allowed, and without it Platter
starts and transcribes on the processor. On the card the processor still makes the
model's input, picks the words and lines them up with the audio: three
threads on `light`, four otherwise (one starved the card), at the same low
priority, and the
busy pause (fullscreen games included) still applies. The lyrics check uses
the transcription helper while on the card, so only one model is ever loaded
on it and runs take turns. macOS works the same way with Metal (the card
there is the Mac's own GPU).

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

### Transcriptions on the user's other devices

With `syncTranscriptions` on (the default; synced), results travel through
the sync store's `transcripts` namespace, keyed `<track id>|<model>`, value
`{ text, model, part, at }`: the enhanced LRC (empty for "no words found"),
the model name, the song's part key (its file version) and when it was made
(unix seconds). Another device saves them as the song's own lyrics with
`auto` set to the model, so they show as transcribed under the same label and
trust rules without running the model there, and "no words found" is
remembered there too. A result for another part key is ignored, so a replaced
file is transcribed again. Which result a song shows: lyrics the user wrote
or edited always win; then the bigger model (`medium` over `small`); then the
newest. Before transcribing a song, the batch (and a single song) takes a
result made elsewhere for the same file instead of running the model, and
sends `transcribe://done` for it. Results applied from the sync file send
`transcribe://done` too. Forgetting a transcription forgets it on the other
devices. Results made before this was on are added when the app starts.
These travel with the "Lyrics you wrote" sync group (`syncLyrics`).

## Sharing lyrics
| Command | Args | Returns |
| --- | --- | --- |
| `lyrics_share_info` | `{ trackId }` | `LyricsShareInfo` |
| `lyrics_share_reviewed` | `{ trackId }` | – |
| `lyrics_share_publish` | `{ trackId }` | – (resolves once published) |
| `lyrics_share_cancel` | `{ trackId }` | – |

`LyricsShareInfo = { offer, whyNot?, unreleased, payload? }`. `whyNot` is
`off` (`shareTranscriptions` is off), `shared` (already shared, on any
device), `notTranscribed` (the song isn't showing a transcription),
`unreleased` (an unreleased song, by the rule the batch uses, while
`shareTranscriptionsUnreleased` is off), `missingInfo` (no title, artist,
album or length) or `notReviewed`. `payload` comes only with an offer and is
exactly what LRCLIB gets: `{ trackName, artistName, albumName, duration
(seconds), plainLyrics, syncedLyrics }`, the synced lyrics being the
transcription's lines without word times. `unreleased` true means the
dialog adds a warning that sharing publishes the title.

The lyrics view calls `lyrics_share_reviewed` when the user opens the
lyrics editor for the song, or plays 90% of it with the lyrics showing.
Nothing is ever shared automatically or in bulk: `lyrics_share_publish` runs
only from the dialog's Share button. It checks the offer again, asks LRCLIB
for a challenge (`POST /api/request-challenge` → `{ prefix, target }`),
finds a nonce whose SHA-256 of `prefix + nonce` is at or below the target on
its own thread at low priority (stopped by `lyrics_share_cancel`), then
`POST /api/publish` with `X-Publish-Token: prefix:nonce` and the payload. The
User-Agent names Platter and its version. On success the song remembers it
(`lyrics_share` table, and `lyricsShared` in the sync store) and isn't
offered again. Errors change nothing: `network` when LRCLIB can't be reached
or Go offline is on, `invalid` with LRCLIB's reason when it refuses, when it's
busy, when cancelled, or when the offer no longer holds.

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

## Devices
Sync to a device: playlists, albums, artists and Liked Songs copied to a USB
drive, an SD card or a music player.

| Command | Args | Returns |
| --- | --- | --- |
| `devices_list` | – | `DevicesView` |
| `devices_add` | `{ root, name, folder }` | `Device` |
| `devices_choose_folder` | `{ root }` | the picked folder below the drive's top (`/` separated), or null when the picker was cancelled; `invalid` for a folder on another drive |
| `devices_update` | `{ id, patch: { name?, folder?, format?, budgetGb? } }` | `Device` |
| `devices_select` | `{ id, kind: "playlist"\|"album"\|"artist"\|"liked", itemId \| null, title \| null, on: bool }` | `Device` |
| `devices_forget` | `{ id }` | – |
| `devices_sync` | `{ id }` | `SyncStatus` |
| `devices_pause` | `{ on: bool }` | – |
| `devices_cancel` | – | – |
| `devices_status` | – | `SyncStatus \| null` |

`DevicesView = { devices: DeviceView[], drives: Volume[], sync: SyncStatus | null }`

`Device = { id, name, volumeId, volumeLabel, folder, format: Format | null, budgetGb: number | null, selection: { kind, id, title }[], lastSync, lastRoot }`

`DeviceView` is a `Device` plus `root` (where it's plugged in now, or null),
`free` and `total` in bytes.

`Volume = { root, label, id, removable, free, total }`

`Format = "original" | "flac" | "mp3-320" | "mp3-v0" | "opus"`

`SyncStatus = { deviceId, deviceName, phase: "planning"|"copying"|"waiting"|"paused"|"done"|"cancelled"|"failed", total, done, kept, removed, current, bytes, failed: { title, error }[], leftOut: { title, songs }[], leftOutSongs, error: { kind: "full"|"writeProtected"|"accessDenied"|"removed"|"notConnected"|"other", message } | null, notice: string | null, safeToUnplug }`

Events: `devices://progress` → `SyncStatus` (at most four times a second, and
on every phase change), and `devices://changed` (no payload) when the device
list changes.

A device is a drive plus a music folder on it, remembered on this computer
only. `devices_list` offers removable drives that aren't a device yet. A
device is found again by its volume serial number (Windows) or volume UUID
(macOS), or else by the manifest in its music folder. `format` null follows
`deviceFormat`; in a patch, `"default"` sets it back to null. `budgetGb` 0
removes the size limit. The selection keeps the order things were added in.

A sync works out what the device should hold, fills it in selection order up
to the size limit and the free space (a song that doesn't fit is listed in
`leftOut`, and smaller songs after it still get their turn), and compares
that with the manifest `.platter-sync.json` in the music folder, which lists
every file Platter wrote there. Only new or changed songs are copied. A file
Platter didn't write is never written over or removed; a song that would take
its name gets ` (2)`. With `deviceDeleteRemoved` on, Platter's own files for
songs no longer chosen are removed, and folders left empty go too.

Songs go in `Artist/Album/NN Title.ext` (`D-NN Title` on albums of more than
one disc), with names safe on FAT32 and exFAT and paths short enough for
Windows. Each chosen playlist and Liked Songs get an M3U8 file at the top of
the music folder with paths relative to it. Original copies the file as it
is. The other formats copy a file already in that codec and convert the rest
on this computer: FLAC (flacenc), MP3 320 kbps or V0 (LAME), Opus at about
160 kbps (libopus). Converted files keep the original's tags, take the
song's basics from the library where the file has none, and get one front
cover at most 500 pixels across.

Songs come through the same transfer gate as downloads, at background
priority, or from the downloaded file when it is the original. Two songs are
made ahead of the copy at most, each on its own low priority thread. The sync
waits (`waiting`) while playback is still filling its buffers, a fullscreen
game is in front, the computer runs hot or other programs keep the processor
busy. Every file is written under a hidden partial name, flushed, and renamed
into place, so `safeToUnplug` is true once a sync is done or cancelled. A
song that can't be fetched or converted is listed in `failed` and the rest go
on. Writing stops with `error.kind` `full`, `writeProtected`,
`accessDenied` (the system refused, say a read-only folder) or `removed`
(the drive went away mid-sync); what was copied by then stays and is
tracked. `notConnected` means the device isn't plugged in.

Each song is noted in `.platter-sync.journal` next to the manifest as soon
as it's in place, and the journal is folded into the manifest at the next
save or sync, so a drive pulled out or a crash never leaves a song
untracked. Partial files left behind are removed at the start of the next
sync. A song already at its own place that Platter isn't tracking, the same
size as what it would write, is taken back instead of copied again. A
manifest from a newer Platter stops the sync with an error and is left as it
is. A damaged one is kept as `.platter-sync.json.bak`, the songs already
there are matched by name and size, and `notice` says so.

A device can't be a fixed drive's top folder: `devices_add` on a drive that
isn't removable needs a music folder.

## Sound map
Every analysed song as a point on a flat map, placed so songs that sound
alike sit together. The core makes it from the feature vectors song analysis
keeps, on this computer, and caches it in the database, so it works offline.

| Command | Args | Returns |
| --- | --- | --- |
| `soundmap_info` | – | `{ placed, analysed, total, analysing, building, revision }` |
| `soundmap_points` | `{ after: string \| null, limit: number \| null, area: { minX, minY, maxX, maxY } \| null }` | `Chunk` |
| `soundmap_find` | `{ query }` | `{ kind: "song" \| "artist", id, title, sub, x, y }[]` |
| `soundmap_song` | `{ id }` | `{ id, title, artist, albumTitle, thumb }` |

`placed` is songs on the map, `analysed` songs with an analysis and `total`
songs that can be analysed (an analysis the map can't read is left out of
`analysed`). `analysing` is true while Automix is on. `revision` goes up each
time the map changes while the app runs, so the same `placed` and `revision`
mean the points already loaded are current. Asking
for the info also places any songs analysed since the last update.

`Chunk = { ids, x, y, genre, genres, year, unreleased, next }` holds the
points in columns, one entry per song in each, in song id order. `genre`
indexes `genres` (-1 for none), `year` is 0 when unknown, and `unreleased`
follows the archive release status. Pass `next` back as `after` for the next
chunk; it's null after the last. `limit` defaults to and tops out at 20,000.
With `area` only points inside it come back. Most of the map sits within -1
to 1 on both axes.

`soundmap_find` returns up to three artists (placed at the middle of their
songs, `sub` is "N songs") and six songs (`sub` is the artist). The search is
plain text, so `%` and `_` match themselves.

Event `soundmap://changed` (no payload) fires when points were added, moved
or removed. Songs analysed later are placed with the projection already in
use, so the map holds still; it's worked out again, moving every point, when
the number of analysed songs has grown by half, and then turned to match the
old layout so the map doesn't flip over.

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
| `offer`, `shared` | host | the guest's down |
| `plexUser` | guest | the guest's up |

Let them listen from my server: `listen_offer_server { personId }` (host
only, while `shareOfferServer` is on) sends that guest `offer`. Their
Platter answers `plexUser` with its Plex username, or none while
`shareAcceptInvites` is off there; the username goes only inside this
sealed message, and only when asked. The host shares its default libraries
through Plex (see Sharing below) and, when the invite went out or they
already have access, sends `shared`, and the guest looks for the invite.
The host's view gives each guest `offer: { state: "asking" | "inviting" |
"invited" | "alreadyShared" | "noInvites" } | { state: "failed", message }`,
and `offerServer` says whether the button shows. After a guest accepts and
the host's libraries are read, it looks for each song again, so the host's
own copy plays.

ntfy as used: `POST <relay>/<topic>` with the sealed text and
`X-Firebase: no`, and `GET <relay>/<topic>,<topic>/json[?since=<id>]`, read
line by line. A 429 shows as `limited`, anything else that fails as
`unreachable`; both retry with a growing wait.

### Sharing

Sharing goes through plex.tv only (docs/PLEX-API.md, "Sharing a server"),
of music libraries only.

| Command | Args | Returns |
| --- | --- | --- |
| `sharing_overview` | – | `{ serverName, owner, plexPass: bool \| null, libraries: { id, title, unreleased: number \| null, default }[] }`, music libraries only; `unreleased` is null for one Platter hasn't read |
| `sharing_share` | `{ invitee, options: { libraries, hideUnreleased, allowDownloads } }` | `ShareStatus`: `{ status: "invited", hidden }`, `{ status: "alreadyShared" }`, `{ status: "needsPlexPass", withUnreleased: string[], others: string[] }`, `{ status: "unknownUser" \| "refused" \| "plexDown", message }`; `invalid` for anything but music libraries on a server you own |
| `sharing_people` | – | `{ id, name, libraries, allowDownloads, unreleasedHidden, pending, lastSeenAt }[]` |
| `sharing_remove` | `{ id }` | – |
| `sharing_link` | `{ options }` | `{ link, qr, expiresAt }`: `platter://share/<token>` (the invite token), one person, once |
| `sharing_open_link` | `{ link }` | `{ status: "asked" \| "invited" \| "alreadyShared", host } \| { status: "failed", message }` |
| `sharing_link_asks` | – | `{ id, user, name, at }[]`: people who opened your share links, waiting for a yes |
| `sharing_link_approve` | `{ id }` | the result in words: shares with them as the link was made |
| `sharing_link_deny` | `{ id }` | – |
| `sharing_invites` | `{ fresh? }` | `ReceivedInvite[]`: `{ id, from, servers, friend, home, server, createdAt }`, server invites only |
| `sharing_accept` | `{ id }` | `{ servers, libraries }`: their music libraries are added to `libraries` and read |

With `hideUnreleased`, the chosen libraries' unreleased albums get the Plex
label `platter-unreleased` and the share's music filter is
`label!=platter-unreleased`; that needs Plex Pass, and without it nothing is
shared (`needsPlexPass`). A failed label write shares nothing.

Events: `sharing://invites` (`ReceivedInvite[]`, when the list changes; new
ones also get a desktop notification while `shareAcceptInvites` is on),
`sharing://notice` (`{ message }`, someone used your share link) and
`sharing://link-asks` (the waiting requests, when they change).

A share link: the friend's Platter seals `{ n, u: <Plex username>, name, x }`
for the link's `share up` topic; the host's answers `{ n, s: "asked" |
"used" | "expired", host }` on `share down`. Nothing is shared until the
host says yes, and the answer is the same whatever username was sent. A link
holds three requests at once and closes after three shares that fail.
`deep_link_take` hands a share link over as `shareAsk`.

## Smart playlists, hide, bookmarks and scrobbling

| Command | Args | Returns |
| --- | --- | --- |
| `smart_preview` | `{ rules: SmartRules, picks: string[] \| null, tzOffsetMin? }` | `SmartPreview` |
| `smart_save` | `{ request: SmartSaveRequest }` | `SmartSaved` |
| `smart_list` | – | `SmartSummary[]` (Platter smart playlists) |
| `smart_page` | `{ id, tzOffsetMin? }` (a `smart:` id) | `SmartPage` |
| `smart_delete` | `{ id }` (a `smart:` id) | – |
| `smart_plex_rules` | `{ id }` (a Plex smart playlist) | `SmartRules`, or `invalid` when its rules don't map to the editor |
| `smart_prompt` | `{ prompt }` | `{ title, rules: SmartRules, picks: Track[] }` |
| `smart_describe` | `{ text, tzOffsetMin? }` | `{ title, rules: SmartRules, unknown: string[], choices: { text, index, others: SmartRule[] }[], notes: string[] }` |
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
1990), `addedAt`, `lastPlayed`, `playedHere`, `played` (inLast, notInLast,
with `30d`, `2w`, `6m` or `1y`; or between, with two local days like
`2026-06-01..2026-08-31`), `liked`, `explicit`, `unreleased`, `lossless`,
`finished` (is, `"true"` or `"false"`), `era` (like `artist`), `skips` (like
`playCount`) and `releaseType` (is, isNot, a release section key). `played`
is any play in the listen history, `skips` counts skips there, and `finished`
means every song on the album has been played. `tzOffsetMin` (minutes east of
UTC) is the time zone between ranges are read in. Sorts: `none`, `random`,
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
can't express (`playedHere`, `played`, `duration`, `releaseType`,
`explicit`, `version`, `era`, `unreleased`, `lossless`, `skips`, `finished`,
a between range, a limit in hours, a daily or weekly update), prompt picks, or
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

`smart_describe` reads a description on this computer and sends nothing
anywhere. Words are matched against the library's own genre, style, mood,
artist, album, era and version names (case, accents and hyphens folded),
then against fixed phrases for dates ("2016", "the 90s", "last summer"),
plays ("haven't played lately", "never skipped"), length ("under 3
minutes"), quality, release type and released or unreleased. Words that
match nothing come back in `unknown`. When words could mean more than one
rule, `choices` gives the other readings for the rule at `index`. The editor
shows the rules before anything is saved.

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

## Shared playlists and Blends

Collaborative playlists and Blends between people who each play from the
Plex servers they already reach. There is no Platter server: everything goes
through the listen along relay (`listenRelay`, ntfy.sh by default), sealed
the same way (`src-tauri/src/social/circle`). Only metadata travels: song
identities (title, artist, album, length, MusicBrainz recording and release
group ids, the song's server-qualified id `<tag>:<rating key>`, the tag being
the hash share links carry), names, roles, Plex profile pictures as `/ext/`
art paths, and small taste summaries. Never audio, a file path, a part key, a
Plex token, a machine id or a server address. Unreleased songs stay out of
everything sent, and out of Blends, unless `shareUnreleased` is on. Both
features are on (`collabPlaylists`, `blends`, synced) but nothing happens
without an invite.

Commands:

| Command | Args | Returns |
| --- | --- | --- |
| `circle_state` | | `CircleView` |
| `circle_people` | | `CircleAccount[]`: people on your server (owner only) and in your Plex Home |
| `circle_join` | `link` | `JoinResult` |
| `circle_accept` | `id` (an invite that came straight to you) | `JoinResult` |
| `circle_dismiss` | `id` | |
| `collab_info` | `playlistId` | `SharedInfo \| null` (your playlist; null when not shared) |
| `collab_invite_link` | `playlistId, role` | `InviteLink` |
| `collab_invite_person` | `playlistId, key, role` | |
| `collab_page` | `id` (`shared:<group>`) | `SharedPage` |
| `collab_add` | `id, trackIds` | |
| `collab_remove` | `id, keys` | |
| `collab_leave` | `id` | |
| `collab_set_in_library` | `id, on` | |
| `collab_set_role` | `playlistId, personId, role` | |
| `collab_remove_person` | `playlistId, personId` | |
| `collab_set_public` | `playlistId, public` | |
| `blend_create` | `keys` (people picked directly, at most 9) | `BlendCreated` |
| `blend_invite_link` | `id` | `InviteLink` |
| `blend_invite_person` | `id, key` | |
| `blend_page` | `id` (`blend:<group>`) | `BlendPage` |
| `blend_hide` | `id, trackId` | |
| `blend_refresh` | `id` | `BlendPage` |
| `blend_leave` | `id` | |

`role` is `"collaborator"` or `"listener"`. Types are in `src/lib/types.ts`
("shared playlists and Blends"). `CircleView = { relay, playlists:
SharedSummary[], blends: BlendSummary[], incoming, joining }`. A
`SharedSummary` carries `role` (`owner`, `collaborator`, `listener`, or
`follower` for someone who left a public playlist and still sees it),
`people` (the owner first, each with `added`, the songs they added),
`public`, `gone` (deleted, or you were taken off), `inLibrary`, `waiting`
(your edits the owner hasn't confirmed) and `ownerUnreachable` (sent three
times without an answer).

Events: `circle://changed` → `CircleView` whenever anything changes (a
member's edit landing, a role, the relay); `circle://notice` → `{ message }`
for toasts (an edit the owner couldn't make, a deleted playlist, someone
joining, an invite that came straight to you).

Invites are listen along's tokens (version, random 16-byte id, random
16-byte secret, expiry, a non-default relay) under their own words:
`platter://collab/<token>` and `platter://blend/<token>`. They work once and
run out after `listenInviteHours`. The person who answers first agrees a key
with whoever made the invite (SPAKE2 over the invite's secret) and gets,
sealed with it, the playlist's or Blend's id, their own id, their link's
secret, the owner and their role (a Blend also gets the secret its members
share). Anyone else who answers is refused (`used`); an expired one is
refused (`expired`) for a day after. While the owner is offline the answer
waits: the request is sent again until the invite runs out and the person is
in once the owner comes back (`JoinResult.state: "waiting"`).

People on the same server or Home are invited without a link: the invite
goes to their doorbell, a channel from the server's machine id (which only
people who reach that server know) and their Plex Home uuid or user name.
A doorbell is listened to only while one of the features is on and there's
someone else on the server or Home. It carries an ordinary invite link, so
it still has to be answered like any other.

Each pair (owner and member) has a link: a secret, two channels (`circle
up`, `circle down`), and an outbox. A packet is `{ f, c, fl, m }` (sender,
counter, the lowest counter still unacknowledged, message), as JSON,
deflated, cut into pieces of 2000 bytes, each sealed like a listen along
message (`1.<base64url(nonce, ciphertext)>`, topic as associated data). The
receiver applies each counter once in any order and answers `ack { upto,
also }`. Everything but an ack stays in the outbox and is sent again (after
1 minute, doubling to 30) until acknowledged, since the relay keeps messages
for hours only. Subscriptions start from the last message seen (`since`).

A shared playlist is a normal Plex playlist owned by its creator. Only the
owner's Platter changes it. A collaborator's `edit { n, op: add { songs } |
remove { keys } }` goes to the owner, who checks the role (collaborators
remove only songs they added), finds each song in its own library (the same
id, else by MusicBrainz id or artist, title and length), applies it through
playlist editing (so it queues for Plex like any other edit) and answers
`applied { n, ok, why? }`. Edits apply in the order the owner gets them; the
owner's playlist is what everyone ends up with. Every change to the
playlist (the owner's own included) goes to every member as a `delta { from,
v, add, del, order? }`; a member out of step asks for a `snapshot`. Members
see the songs they can play here; the rest show "Not in your library" and
can go to the Missing list by album. Deleting the playlist sends `gone`;
taking someone off sends `removed`.

A Blend's members share one channel. Each sends a taste summary on it: up
to 15 top artists and 30 top songs over the last 8 weeks with weights (a
play counts in full for a week, then half as much every two weeks older),
and 10 recent likes, from this computer's own listen log (a private session
writes nothing there, so its plays never count; none is sent during one).
It's sent when someone joins and about daily. Each person's Platter makes the
Blend from the summaries and the songs it can play: about 50 songs, members
taking turns from a day's own starting point so each gets about an equal
share, made again each day or when someone new joins. Hidden songs never
come back. Leaving deletes the Blend here and sends `left`; the others drop
the leaver at their next day's Blend, and a Blend left with one person is
removed. Ten people at most, you included.

State is kept in the library cache's `kv` table (`circle.state`).

## Listening stats

| Command | Args | Returns |
| --- | --- | --- |
| `stats_overview` | `{ range: { from, to }, limit? }` | `StatsOverview` |
| `stats_span` | – | `{ first, last, months: string[], today }` |
| `stats_moments` | `{ range }` | `StatsMoments` |
| `stats_days` | `{ from, to }` ("YYYY-MM-DD", at most 400 days) | `{ key, plays, ms }[]` for each day with plays |
| `stats_item` | `{ kind: "song"\|"artist"\|"album", id }` | `ItemStats` |
| `stats_month_recap` | `{ month }` ("YYYY-MM") | `MonthRecap` |
| `stats_play_ids` | `{ range, kind: "songs"\|"artists"\|"albums"\|"genres", limit? }` | track ids, most played first |
| `stats_compare_summary` | `{ unreleasedOk, theirUnreleasedOk }` | `CompareSummary`, or `invalid` while Compare is off |
| `stats_compare` | `{ theirs: CompareSummary, unreleasedOk }` | `CompareResult`, or `invalid` while Compare is off |
| `top_songs_index` | – | `{ years, allTime, promoted, wrappedYears, currentYear }`, or `invalid` while Wrapped and Top Songs are off |
| `top_songs_list` | `{ year? }` (no year: All-Time) | `TopSongsList`; `notFound` for a year without a list |
| `stats_wrapped` | `{ year }` | `Wrapped` |
| `home_on_this_day` | – | `{ shown, date, years: OnThisDayYear[] }`; `shown` is false (and the rest empty) while `homeOnThisDay` is off |
| `home_time_travel` | – | `{ shown, yourYear: HistoryMix \| null, decades: HistoryMix[] }`; `shown` is false while `homeTimeTravel` is off |

Home's On this day lists what was played on today's date in each earlier
year with plays on it: `OnThisDayYear = { year, date, plays, ms, songs, albums, trackIds, covers }`.
`songs` (up to 50) and `albums` (up to 10) are top songs and albums for that
day, most played first. A song no longer in the library has no `trackId`;
`trackIds` holds only library songs, which is what a card plays. Years come
busiest first, ties to the later year. Feb 29 matches only Feb 29; Feb 28 in
a year without one also takes in Feb 29 of earlier leap years.

Time travel has one `HistoryMix = { id, kind: "decade"|"year", year, plays, trackIds, covers }`
per decade the library has music from (by album year, oldest first): up to
50 of the decade's library songs played most, filled up from the songs the
server counts as played most when there are fewer. `yourYear` is the
calendar year with the most plays (ties to the later year) as a mix of its
50 most played library songs, or null when none of them is in the library.
`covers` are up to four album covers for the card.

Both read the stats day tables, so a play the listen log and an import both
know about counts once. Each answer is kept until the day changes or the
counted plays do.

Stats count every play that wasn't a skip, from the listen log and imported
plays together (see Play history from before Platter and other apps below). A range is two local dates ("YYYY-MM-DD"), both included; no
dates means all time. Plays are summed per local day for each song, artist,
album and genre, and per day and hour. Changing, adding or removing a play
marks its day, and the next stats call counts only the marked days again.

`StatsOverview = { from, to, totals: { ms, plays, songs, artists, albums, days }, bucket: "day"|"month", series: { key, plays, ms }[], streak: { days, from, to } | null, hours: number[24], weekdays: number[7], busiestHour, busiestWeekday, topSongs, topArtists, topAlbums, topGenres }`.
The chart is by day for ranges under 92 days and by month for longer ones.
Weekdays start on Sunday. Top lists rank by plays, then minutes. A top song
is `{ key, trackId, title, artist, album, plays, ms, unreleased, track }`;
`track` is the library song when it's still there.

A top genre also has `thumb`, the cover of the range's most played album
with songs in it. `MonthRecap` has `topAlbums`, up to four, for its covers.

`StatsMoments = { firstSong, replayed, discovery, comeback, newArtists }`.
`firstSong` is the range's first play. `replayed` is the day one song was
played most, at least 3 times. `discovery` is the artist first played in the
range that you played most there, and `newArtists` how many artists were new
(null for all time, where both are left out). `comeback` is the song played
again after the longest break, at least 30 days. The songs' plays and
minutes are that day's.

`ItemStats = { kind, plays, ms, first: { at, song, trackId, from: { kind, id, title } | null } | null, lastAt, rank, series }`.
`rank` is the place in your all-time top for that kind, `series` the plays
per month for the last 12 months.

`CompareSummary = { version, name, artists: { name, weight }[], songs: { artist, title, weight }[], unreleasedOk }`
is all that's shared with a friend: up to 50 artists and 50 songs from about
the last six months, each with its share of the plays. Unreleased songs are
only in it when both sides agreed. `CompareResult = { friend, overlap (0 to 100), commonArtists, theirSongsNew }`.

Years for Wrapped and Top Songs are local calendar years. A year has a Top
Songs list once it has 30 plays and is over enough: past years always, this
year from December 1. `years` and `wrappedYears` are newest first;
`wrappedYears` has every such year with any plays. `promoted` is the year
Home shows: this year's list through December, last year's until January 15,
null otherwise (and when that year has no list).

`TopSongsList = { year, title, rows: { track, plays, ms, unreleased, firstOfYear }[], durationMs }`:
up to 100 library songs, ranked by plays then minutes; the position is the
rank. Each version is its own song, credited to the file played.

`Wrapped = { year, totals, prevMs, topArtists, topSongs, topAlbums, topGenres, personality, hours, biggestDay, replayed, firstSong, newArtists }`,
with up to five of each top list. `personality = { time: "morning"|"afternoon"|"evening"|"night", timeShare, repeatShare, repeats, newShare, mostlyNew } | null`:
the part of the day with the most minutes (morning 5 to 12, afternoon 12 to
5, evening 5 to 10, night 10 to 5) and its share; the share of plays on
songs played 5 times or more that year (`repeats` from half, otherwise deep
cuts); and the share on songs first heard that year (`mostlyNew` from
half). Unreleased songs count and carry `unreleased`; the summary card
leaves them off unless the person turns that on.

### Play history from before Platter and other apps

Stats, mixes (On Repeat included), recaps and smart playlists read every play
from one place, `db::listens::ALL_PLAYS`: the listen log plus
`imported_plays`. Besides history files (the import page), imported plays come
from three sources, each on its own once set up and while its setting is on:

- `plexHistory`: the music section's history on the connected Plex server
  (`/status/sessions/history/all`), the signed-in user's account only. The
  account is the one named like the user in the server's `/accounts`, or the
  owner's (1) on a server they own; it is asked for with `accountID` and
  every entry is checked again, so other Home users' and friends' plays never
  count. When the account can't be told, nothing is read. Each entry maps to
  its song by rating key; when the song is gone, by artist, title and album
  (as history files are matched), else it's kept by its names.
- `tautulli`: Tautulli's `get_history` for the user's music plays (Tautulli's
  `user`, the Plex user name). Plays too short to count (under 30 seconds and
  not heard to the end, Platter's own skip rule) are left out.
- `lastfm`: the connected account's `user.getRecentTracks`, matched to library
  songs by artist, title and album. Scrobbles that don't match are kept by
  their names and still count where no library song is needed (top artists).

The first read of a source takes the whole history in the background, 200 at
a time with a second between pages, and can be stopped (it goes on from the
same page). Later reads take only what's newer than the newest play already
brought in, after each library sync and every 6 hours. A play is saved once
per source id (the history entry id, Tautulli's row id, Last.fm's time and
song), so reading again adds nothing.

Nothing counts twice: an imported play of the same song within 3 minutes of
a local play (before it started or after it ended) is that play reported
elsewhere, and the same play from several sources counts once, from the
first of Plex history, Tautulli, Last.fm, a file (same song within 3 minutes
plus its length; unmatched ones by title and artist). Both rules live in the
view, so they hold whichever play arrived first.

| Command | Args | Returns |
| --- | --- | --- |
| `play_history_status` | – | `{ sources: PlayHistorySourceStatus[] }` (Plex, Tautulli, Last.fm in that order) |
| `play_history_update` | – | the status; every source set up and turned on is read now, failed ones too |
| `play_history_cancel` | – | the status; the read going on stops after its page and goes on later |
| `play_history_remove` | `{ source: "plexHistory"\|"tautulli"\|"lastfm" }` | the status; deletes only that source's imported plays. Nothing comes in from it again until its setting is turned off and on (the UI turns it off) |
| `tautulli_status` | – | `{ urlSet, hasKey }` |
| `tautulli_key_set` | `{ key: string \| null }` | the same, after saving the key in the OS keychain (`tautulli-api-key`) or removing it; `invalid` unless letters and numbers. Never returned, logged or put in settings |

`PlayHistorySourceStatus = { source, setUp, included, plays, notInLibrary, shortPlays, updatedAt, firstReadDone, importing, done, total, error, removed }`.
`setUp` is a Plex server with a music library, Tautulli's address and key,
or Last.fm connected for scrobbling. `plays` counts what was brought in
(kept while `included` is off, but not counted anywhere), `notInLibrary` the
ones not matched to a library song. While `importing`, `done` and `total`
say how far it is. `error` is why the last read stopped, in words for
Settings; it's tried again 30 minutes later or on the next sync.
`playhistory://status` carries the same status whenever a read starts, moves
on a page or ends.

Settings (all synced): `includePlexHistory`, `includeTautulliHistory` and
`includeLastfmHistory` (on; off leaves that source's plays out of every count
at once and stops reading it), and `tautulliUrl` (empty: Tautulli isn't set
up). A first listen's `from` in `stats_item` is `{ kind: "imported", id }`
for an imported play, `id` naming the source (`plex_history`, `tautulli`,
`lastfm` or `file`), shown as "from your Plex history", "from Tautulli",
"from Last.fm" or "in your imported history".

## Lidarr

Lidarr's home and away addresses are the `lidarrUrl` and `lidarrAwayUrl` settings. The API key and any extra proxy headers are in the OS keychain and never cross back to the UI. The check only reads from Lidarr. Every write is one of a fixed set (`lidarr/write.rs`): adding an artist (only with a fresh check of Lidarr's import list exclusions by MusicBrainz id; Managed artists into a library root, monitored with `monitorNewItems` and `addOptions.monitor` set to `none` and no search; protected artists into the staging root, unmonitored), monitoring albums and `AlbumSearch`; and, only after the user said yes to the exact change, adding or removing exclusions, one value on a settings page, the staging root folder and the "Platter Hi-Res" profile; freeing a promoted album's files inside the staging folder; and a manual import onto an album's current release. Nothing triggers a global rescan. Before a request the release group is checked on MusicBrainz again and the whole setup check runs again: a red finding that stops everyone (no root folder) stops it, and any red finding stops a request for a protected artist.

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
| `lidarr_request_batch` | `{ inputs: LidarrRequestInput[] }` | `{ releaseGroupId, title, record, error }[]`. Up to 10 albums the user confirmed at once, each sent like `lidarr_request` (full check and every guard), one after another; one that fails doesn't stop the rest. More than 10 is refused before anything is asked |
| `lidarr_requests` | `{ refresh: boolean }` | `LidarrRequest[]`, newest first; `refresh` reads their state from Lidarr's queue and history |
| `lidarr_get` | `{ input: LidarrRequestInput }` | `{ record: LidarrRequest \| null, needs: { kind: "firstTime" \| "refused", reason } \| null }`. One-click Get: `lidarr_request` with every check. `needs.kind` is `firstTime` when no request was ever made (the dialog asks for the typed name) and `refused` when MusicBrainz or a check says no (`invalid` and `notFound` errors), with the reason; nothing is sent then |
| `lidarr_search_again` | `{ id }` | `LidarrRequest`. For a request that's `searching` or `notFound`: one `AlbumSearch` for that album now, and the searches again start over. Needs a passed setup check and the album still monitored |
| `lidarr_request_cancel` | `{ id }` | `LidarrRequest[]`; only waiting or finished requests, else `invalid` |
| `lidarr_missing_releases` | `{ artistId, artist }` | `{ shown, items: { releaseGroupId, title, kind: "album"\|"ep", year, request, thumb }[] }`; `thumb` is the small front cover as an art path (`/ext/https://coverartarchive.org/release-group/<id>/front-250`), which fails to load when the archive has none; `shown` is false unless the setting is on, Lidarr is set up and its last check passed |

`LidarrReport = { version, findings: LidarrFinding[], passed, managedOk, protectedCount, checkedAt, route: "home"|"away"|null }` and `LidarrFinding = { id, level: "red"|"amber"|"ok", title, detail, fix: string|null, emptyRoot: { path, confirmed }|null, protectedOnly, action: LidarrFixAction|null }`.
Findings come red first, then amber, then OK. `passed` is false while any finding is red (protected work waits); `managedOk` is false only for a red finding with `protectedOnly` false. `LidarrFixAction` is `{ kind }` with `addExclusions { artists: { name, mbid }[] }`, `setRecycleBin` (needs a folder), `writeTagsOff`, `renameOff`, `addStagingRoot { path }`, `rematch { folders }` or `releaseSwitchOff { albums: { id, title }[] }` (turns off "Automatically Switch Release" for Managed albums whose folders hold files Lidarr doesn't track; each album is written back exactly as Lidarr sent it with only `anyReleaseOk` off, and can't be put back from Platter). `renameTracks` is amber only while a protected artist's folder is under a Lidarr root folder.
The home address is tried first (for 4 seconds when an away address is set), then the away address.

`LidarrRequestInput = { releaseGroupId, title, artist, artistId?, typed? }`.
`LidarrPreview = { releaseGroupId, title, artist, destination, destinationIsGuess, needsTyped, existing, reachable, route, addsArtist, protected }`; `protected` means it lands in the staging folder and waits there until Platter adds it to the library.
`LidarrRequest = { id, releaseGroupId, title, artist, artistId, state, destination, lidarrAlbumId, addedArtist, alreadyInLidarr, createdAt, updatedAt, message, notes, auto, protected, grabbed }`, where `grabbed` is `{ line, upgradeNote } | null`: once imported, what Lidarr got ("FLAC, 13 tracks", "MP3 320, Newsletter Edit") and, for a lossy copy under a profile that keeps upgrading, "Lidarr will replace it with FLAC when one turns up.", with `state` one of `queued`, `requested`, `searching`, `downloading`, `imported`, `failed`. `auto` is true for a request auto-request made; one it refused stays as `failed` with why in `message`.
Requests are kept on this device. Waiting ones are retried every 5 minutes in the background, full check first.

Requests take official studio release groups only (no live, compilation or other secondary type), never an artist on Lidarr's import list exclusions (matched by MusicBrainz id) and, when the user limited an artist to chosen releases, only those. A release group Plex already matched in the library (`mbid`) is never requested. After adding an artist Platter reads its path back: a protected artist's must be inside the staging folder and outside every Plex music folder; then the whole check runs again before anything is searched.

### Managed and protected artists

Each artist is Managed (Lidarr owns its folder, imports, renames and upgrades in place) or Protected (curated: Lidarr only downloads into the staging folder and Platter adds albums to the library itself). Auto picks Protected when any file's release status isn't released (`tags::provenance`), an album is typed demo, unreleased, leak or bootleg, the artist is in the library and on Lidarr's import list exclusions, or its albums sit inside a protected artist's folder or share a release group found there (Lidarr artists credited on those release groups too). Choices, the catch-up cursor and claims, the changes log, staged albums and better-copy memory are shared through the sync store (`lidarr` namespace, in the Library group).

| Command | Args | Returns |
| --- | --- | --- |
| `lidarr_modes` | – | `LidarrArtistMode[]`, protected first |
| `lidarr_mode_set` | `{ key, name, mode: "auto"\|"managed"\|"protected", confirmations }` | `LidarrArtistPrefs`; `invalid` for `managed` with fewer than 2 confirmations (the UI asks twice) |
| `lidarr_artist_choices` | – | `Record<key, LidarrArtistPrefs>` |
| `lidarr_artist_choices_set` | `{ key, name, autoAdd: boolean\|null, hunt, target: "lossless"\|"hiRes" }` | `LidarrArtistPrefs` |
| `lidarr_change_preview` | `{ action: LidarrFixAction, input: string\|null }` | `LidarrProposal = { id, what, before, after, canPutBack }`; reads only. `input` is the folder for `setRecycleBin` (refused inside a Plex music folder or a root folder) |
| `lidarr_hires_preview` | – | `LidarrProposal` for the "Platter Hi-Res" profile (24-bit lossless only, cutoff FLAC 24bit) |
| `lidarr_change_apply` | `{ id }` | `LidarrChange`; refused when the proposal is older than 15 minutes or Lidarr's value changed since it was shown |
| `lidarr_changes` | – | `LidarrChange[]`, newest first (up to 300) |
| `lidarr_change_put_back` | `{ id }` | `LidarrChange` with `putBackAt`; a root folder holding artists or another root, or a profile in use, is never removed |
| `lidarr_rematch_preview` / `lidarr_rematch_apply` | `{ folder }` / `{ folder, releaseId }` | `LidarrRematchPlan = { folder, albumId, album, release, releaseId, files, kept: { path, why }[], otherEdition, linkedNow, editionNote }`. Only with Rename Tracks off. Imports only onto the album's current release with explicit track ids (matched by disc and track number, tracks without a file), never switching the edition; afterwards the album must have exactly that many more files and every file it had, else `server`. Apply stops with `invalid`, importing nothing, when the album isn't on `releaseId` any more or Lidarr switches it before the import |
| `lidarr_work` | – | `LidarrWork = { catchUp: { lastDevice, lastAt, gap, paused }, practiceUntil, practice, staged: LidarrStaged[] }` |
| `lidarr_catch_up` | – | `LidarrCatchUpReport = { handled, elsewhere, settling, acted, gap, scanned, announced, staged, healed, paused }` |
| `lidarr_resume_protected` | – | – (keeps Lidarr's settings as they are now) |
| `lidarr_staged_add` | `{ id, always }` | `LidarrStaged`; `always` adds the artist's later albums on their own |
| `lidarr_staged_undo` | `{ id, trashOk }` | `LidarrStaged`; `invalid` asking about Plex's "Empty trash automatically after every scan" when it's on and `trashOk` is false |
| `lidarr_hunt` | – | `LidarrHuntItem[] = { albumId, title, artist, now, state, note }[]`, `state` one of `atTarget`, `requested`, `waiting`, `partlyUpgraded`, `outOfScope`, `needsProfile`, `failed` |

`LidarrArtistMode = { key, name, artistId, mbid, lidarrId, lidarrPath, folder, mode: "managed"|"protected", setting, auto, line, reasons, files, curatedFiles, excluded }`; `line` is one sentence like "Protected: 403 files tagged unreleased or leaked".
`LidarrArtistPrefs = { mode, autoAdd: boolean|null, hunt, target, name }`.
`LidarrChange = { id, at, device, what, before, after, asked, undo, putBackAt, practice, byLidarr }`; `byLidarr` marks a change Lidarr made on its own that Platter noticed (an album switched to another edition between two reads, logged as "Lidarr switched <album> to the <n>-track edition"; Platter does nothing about it); `asked` is false for writes to albums Platter requested itself (monitoring, freeing staging), which don't ask but are logged; `practice` marks what practice mode would have done.
`LidarrStaged = { id, albumId, lidarrArtistId, artist, title, releaseGroup, partial, state: "waiting"|"ready"|"practice"|"added"|"undone", reason, plan: { folder, files: { src, name, edits: [tag, value|null][] }[] }|null, folder, confident, doubts, auto, at, updatedAt }`.

The catch-up (`lidarr/catchup.rs`, no UI dependencies) runs on start, every 4 minutes and after the computer wakes: it reads `GET /api/v1/history/since?date=` from the last handled event (shared per Lidarr), claims each event in the shared store first (the earliest claim wins and must stand a minute before its device acts), and acts on `downloadImported` (Managed: a scan of just the album's folder when Plex doesn't show it within about two minutes, then an announcement; Protected: queued for adding) and `albumImportIncomplete` (never added). When Lidarr's oldest kept record is newer than the last one handled, it says some activity may be missed and scans Managed artists' folders Plex has nothing from. Each pass unmonitors protected albums Lidarr no longer needs and re-reads Rename Tracks, Write Audio Tags and the recycling bin; a change toward unsafe while artists are protected pauses protected work (Rename Tracks counts only while a protected artist's folder is under a Lidarr root folder).

Adding a protected album copies it into the artist's folder with the folder, file names and tags its official albums use (`lidarr/conventions.rs`), never touching existing files, then unmonitors it in Lidarr, then (with `lidarrFreeStaging` on and only once it's unmonitored) frees its staging copy, then scans that folder in Plex. The first album per artist and anything from low-confidence conventions always ask. Practice mode (`lidarrPracticeUntil`: -1 the first week after setup, 0 off) logs automatic adds and swaps instead of making them.

### Upgrades

Better copies of albums already in the library. Off until `lidarrStagingFolder` (Lidarr's path for a root folder Plex doesn't scan) is set. An album is upgraded only after the user confirms which MusicBrainz release group it is. The request is a normal request with the same three writes, into the staging root only, with an existing quality profile whose cutoff is above the album's lowest-quality released song (Platter never creates or edits profiles). Swaps run on this computer through the music share mapping, never through Lidarr: the new copy takes the old file's name (new extension), folder and every tag, the source note gets `Upgraded to <quality> from Lidarr, <date>`, the old file moves to the backup place, then Plex rescans just that folder. Any failure puts every file back.

A file swaps when it and the new file carry the same ISRC, or when all of these hold: a tag says it's the released recording, its source note starts with `upgradeOfficialPrefix`, it's on the confirmed release's tracklist, the lengths are within a second, and the decoded audio correlates at 0.99 or more. A length more than a second off the release's track is another version and never swaps. Everything else is kept, with the reason. Release status and the source note come from `lidarr::provenance::StatusReader`, which uses the library's provenance reader (`tags::provenance`: ARCHIVE_VERSION, a plain STATUS tag, Picard's release status and type, or the custom tag); the note is a plain VERSION tag, else ARCHIVE_SOURCE, and the swap appends to whichever the file uses.

| Command | Args | Returns |
| --- | --- | --- |
| `lidarr_upgrade_album` | `{ albumId }` | `AlbumUpgrade = { available, record: UpgradeRecord\|null, quality: string\|null, confirmed: UpgradeConfirmed\|null }`; `available` when the staging folder is set, the last check passed and the album is released and below 24-bit |
| `lidarr_upgrade_candidates` | `{ albumId }` | `{ title, artist, songs: { disc, track, title, durationMs }[], confirmed, candidates: { group, tracks: { disc, position, title, lengthMs }[] }[] }`: official studio release groups with the album's title, each with its first official release's tracklist |
| `lidarr_upgrade_confirm` | `{ albumId, releaseGroupId: string \| null }` | `UpgradeConfirmed = { releaseGroupId, title, at }`, or null after forgetting; `invalid` for a group the request rules refuse |
| `lidarr_upgrade_request` | `{ albumId }` | `UpgradeRecord`; `invalid` without a staging folder, before the release is confirmed, for an unreleased album, a red check, an artist Lidarr keeps outside staging, an excluded artist, or when no quality profile aims higher (said plainly). Kept waiting when Lidarr can't be reached |
| `lidarr_upgrade_bulk` | `{ under: "lossy" \| "belowCd" }` | `{ requested: UpgradeRecord[], skipped: { title, reason }[], more }`: confirmed albums below the quality, up to 25 a run |
| `lidarr_upgrades` | `{ refresh: boolean }` | `UpgradeRecord[]`, newest first |
| `lidarr_upgrade_offer` | `{ id }` | `UpgradeOffer` (below). Reads Lidarr (GET only) and the files |
| `lidarr_upgrade_swap` | `{ id, trackIds: string[] \| null }` | `{ record, swapped, backupDir, summary }`. `null` swaps every song that upgrades and is refused when the album needs asking per song. `invalid` with the failure's words otherwise; the record keeps `failure` |
| `lidarr_upgrade_undo` | `{ id }` | `UpgradeRecord`; the old files back byte for byte, the new copies moved to the backup's `Replaced` folder |
| `lidarr_upgrade_clear` | `{ id }` | `UpgradeRecord[]`; not while a swap can be undone |
| `lidarr_allowlist` / `lidarr_allowlist_set` | `{ artist, artistId? }` / `{ artist, artistId?, releaseGroupIds }` | `string[]`: the release groups an artist is limited to (empty: no limit) |
| `music_share_get` / `music_share_set` | – / `{ map: ShareMap }` | `ShareMap = { music: { server, local }[], staging, backup }`, this device only. `backup` empty means a `_to_delete` folder in the library root, with `_to_delete/*` added to its `.plexignore` when missing; a chosen folder must be outside every music folder |
| `music_share_plex_folders` | – | Plex's music folders as Plex sees them |

`UpgradeRecord` is a `LidarrRequest` plus `{ albumId, fromQuality, swaps: SwapRun[], failure: UpgradeFailure|null }`; `SwapRun = { id, at, backupDir, files, undone, summary }`.
`UpgradeOffer = { upgradeId, albumId, title, artist, status, message, summary, from, to, upgrade: OfferSong[], kept: OfferSong[], askEach, askReason, ask, backupDir, lastSwap, failure, failureMessage }` with `status` one of `waiting`, `needsMapping`, `unreachable`, `ready`, `nothing`, `swapped`, and `OfferSong = { trackId, title, disc, track, from, to, reason, keep, by: "isrc"|"sound"|null }`. `askEach` is true when anything in the album's folders is kept for a reason other than "the new copy isn't better"; the swap then asks per song whatever `upgradesAsk` says.
`UpgradeFailure` is `{ kind }` with `noMapping`, `shareUnreachable { path }`, `changed { name }`, `stagingMissing { name }`, `fileLocked { name }`, `nameTaken { name }`, `tagCopyFailed { name, why }`, `backupNotWritable { path }`, `backupInLibrary { path }`, `moveFailed { name }`, `rescanFailed`, `plexUnreachable` or `backupMissing { name }`. Each leaves the old files in place.
With `upgradesAsk` off, the background loop swaps imported upgrades on its own when the album doesn't need asking per song.

### Platter Bridge
Optional: the home side on a server (docs/BRIDGE.md). Settings > Bridge.

| Command | Args | Returns |
| --- | --- | --- |
| `bridge_overview` | – | `BridgeOverview` |
| `bridge_pair_start` | – | `BridgePairStart` |
| `bridge_pair_cancel` | – | – |
| `bridge_unpair` | `{ id: string }` | `BridgeOverview` |
| `bridge_queued_apply` | `{ id: string }` | `BridgeOverview` |
| `bridge_queued_dismiss` | `{ id: string }` | `BridgeOverview` |
| `bridge_send_settings` | – | – |

`BridgeOverview` is `{ bridges, pairing, queued }`: each paired Bridge
(`id`, `name`, `version`, `pairedAt`, `lastSeen`, `online`, `music`:
`none | readOnly | readWrite`, and `status`, what it last said: `doing`,
`lastCatchUp`, `problem`), the pair code waiting to be used (`code`,
`expiresAt`), and Lidarr settings changes a Bridge queued instead of making
(`what`, and the exact `method`, `path` and `body`). `bridge_queued_apply`
makes one from this computer after the yes, through the same checks;
`bridge_unpair` tells the Bridge too. Event `bridge://changed` (no payload):
read the overview again. Lidarr routes now include `bridge` (the setup
check's and a request preview's `route`).

## Release radar

New official albums, EPs and singles from followed artists and the ten artists played most in the last 90 days. About once a day, when MusicBrainz is on, Platter asks MusicBrainz's release group search for a few artists at a time (by MusicBrainz id, first released in the last 90 days or later), through the shared one-a-second limiter, with answers cached for 18 hours. Only release groups that pass the same rules as a request are kept: at least one official release, album, EP or single, credited to the artist alone, no secondary type. What was found is kept on this device. An artist's first look announces nothing.

Auto-request is per artist (and the `autoRequest` setting for all of them). It asks Lidarr for new albums found after it was turned on for the artist, oldest first, at most `autoRequestCap` a day (refused ones don't count, the rest wait for the next day), through the same path as `lidarr_request` without the typed name: MusicBrainz guards, then the whole setup check. A release asked for once is never asked for again. A summary notification follows.

| Command | Args | Returns |
| --- | --- | --- |
| `release_radar` | – | `RadarPage` |
| `release_radar_check` | – | `RadarPage` after a look now; `network` when MusicBrainz can't be asked |
| `radar_auto_artists` | – | `{ id, name, enabledAt }[]` |
| `radar_auto_set` | `{ id, name, on }` | the same list; turning one on also turns the `autoRequest` setting on (and sends `settings://changed`) |

`RadarPage = { shown, items: RadarItem[], checkedAt, checking, lidarrReady, artists }`. `shown` is false while the `releaseRadar` setting is off.
`RadarItem = { releaseGroupId, title, artist, artistId, kind: "album"|"ep"|"single", date, thumb, libraryAlbumId, request, auto, refusal, new }`, newest first. `libraryAlbumId` is the library album with the same title ("In your library"); `refusal` says why it can't be requested when it shares a title with an album that counts as unreleased (the user's marks included); `request` is the state of a request for it; `new` marks what the latest look found. `thumb` is the Cover Art Archive front cover as an art path.

Event: `radar://changed` (no payload) when a look starts or ends or auto-request changes.

## Discover

Music that isn't in the library, from MusicBrainz (the shared one-a-second limiter, answers kept a week) and ListenBrainz's similar artists (one a second, kept a week). Nothing is asked while Go offline is on or while the setting for it is off.

| Command | Args | Returns |
| --- | --- | --- |
| `catalogue_search` | `{ query }` | `CatalogueSearch`; `shown` is false while `catalogueSearch` is off or offline |
| `catalogue_search_part` | `{ query, part: "artists" \| "albums" \| "songs" }` | `CataloguePart`. One kind, so the UI asks for all three at once and shows each as it arrives. Artists and albums come from Lidarr's own search (`GET /artist/lookup` and `/album/lookup`, no limit) when `catalogueSource` is `lidarr` and Lidarr has an address and a key, and from MusicBrainz when it isn't or fails; songs always from MusicBrainz. Kept 15 minutes per kind, source and query |
| `catalogue_artist_photo` | `{ mbid, name }` | `{ photo, credit }` art path and credit, both null when no source has one. Lidarr's lookup first, then Wikidata's picture on Wikimedia Commons, then TheAudioDB (the user's key, else the free tier). Kept 30 days, a miss 3 days |
| `catalogue_get_albums_plan` | `{ mbid, name, kinds: string[] }` | `GetAlbumsPlan = { artist, artistMbid, kinds, albums: CatAlbum[] }`: the artist's official albums of these kinds (`studio`, `ep`, `live`, `compilation`; studio when none) the library lacks and nobody asked for, most editions first, then oldest |
| `catalogue_get_albums` | `{ mbid, name, kinds }` | `{ queued, plan }`. Works the plan out again and puts it on the waves' queue (see Bring in my listening): sent a few at a time with the same guards and daily pace, each through the full request path |
| `catalogue_artist` | `{ mbid }` | `CatalogueArtistPage`; `notFound` for an unknown id, `network` offline |
| `catalogue_album` | `{ releaseGroupId }` | `CatalogueAlbumPage` |
| `catalogue_fans_also_like` | `{ artistId, name }` | `CatArtist[]` outside the library for a library artist (empty while `fansAlsoLikeCatalogue` is off) |
| `new_to_you` | – | `{ shown, artists: { mbid, name, because: string[] }[] }`; worked out at most twice a day from the artists played most |
| `import_missing_albums` | – | `MissingAlbum[]`: albums added to Missing by hand, with `links` and `libraryAlbumId` once one arrives |
| `import_missing_album_add` | `{ album: { releaseGroupId, title, artist, artistMbid } }` | the list |
| `import_missing_album_remove` | `{ releaseGroupId }` | the list |

`CatalogueSearch = { shown, artists: CatArtist[], albums: CatAlbum[], songs: CatSong[] }`. Anything the library has is left out: artists by MusicBrainz id or folded name, albums by folded title under that artist, songs by folded artist and title. Albums need an official release and to be an album, EP or single not credited to Various Artists.
`CatArtist = { mbid, name, disambiguation, kind, photo, libraryId, photoCredit }`; `libraryId` is set for an artist the library has (open their page instead).
`CataloguePart = { shown, part, source: "lidarr" | "musicBrainz", artists, albums, songs, inLibrary: string[] }`; `inLibrary` holds the library artist and album ids a catalogue result folded into (the "In your library" cue).
`CatalogueArtistPage` also has `popular: CatAlbum[]`, their albums and EPs with the most editions.

`LidarrRequest` states: `queued`, `requested`, `searching`, `downloading` (with `progress` 0 to 100 from the queue's sizes), `importing`, `imported`, `failed` (with the queue's error message when it has one) and `notFound`. While anything is in flight the core reads Lidarr's queue and history every 15 seconds. An album with nothing grabbed gets `AlbumSearch` again 1 hour, 6 hours and 24 hours after the first search, then daily for 7 days (`searchedAt`, `searches`, `nextSearchAt` are kept with the request, so the schedule survives a restart); a day after the last one it's `notFound`. Nothing else is ever sent for it.
`CatAlbum = { releaseGroupId, title, artist, artistMbid, kind, secondary, year, thumb, libraryAlbumId, request, refusal, inMissing }`. `refusal` says why it can't be requested (the same rules a Lidarr request checks).
`CatalogueArtistPage = { artist, area, lifeSpan, bio, bioCredit, bioUrl, banner, sections: { kind: "album"|"ep"|"single"|"other", albums }[], similar, dismissed }`.
`CatalogueAlbumPage = { album, tracks: { disc, number, title, artist, durationMs }[], released, label, links: { bandcamp, qobuz, itunes }, artistLibraryId }`. The tracks are the earliest official release's.

Requests from these pages use `lidarr_request_preview` and `lidarr_request` with `artistMbid` in the input, so the artist is known without a name search. Store links open through `import_open_link`. Artists marked "Not interested" are the synced `dismissedArtists` setting (`{ mbid, name }[]`).

## Played a lot, not in your library

The Missing page's second tab lists what imported listening history (any source in `imported_plays`) played that has no match in the library. Plays count when they lasted 30 seconds or more. Sources left out of every count are left out here too.

| Command | Args | Returns |
| --- | --- | --- |
| `import_played` | `{ query: { kind: "song"\|"album", offset, minPlays, period: "all"\|"year"\|"days90" } }` | `PlayedPage`, 100 rows at a time, most plays first, then most time |
| `import_played_hide` | `{ kind: "song"\|"album"\|"artist", artist, name, hide }` | `PlayedHidden[]`. Hides a row or a whole artist, or brings it back. Kept in the sync store's `playedHidden` namespace |
| `import_played_hidden` | – | `PlayedHidden[]`, newest first |

`PlayedPage = { kind, total, offset, rows: PlayedRow[], hasHistory }`.
`PlayedRow = { kind, key, artist, title, album, plays, ms, firstPlayed, lastPlayed, songs, inLibrary, links, thumb, request }`. Songs group by folded artist and title, albums by folded artist and album. For an album, `songs` is how many of its songs were played and `inLibrary` how many of its tracks the library already has. `request` is the state of a Lidarr request made for the album on this device.
`PlayedHidden = { kind, artist, name, hiddenAt }`.

Before the list loads, and after every library sync, plays whose song the library now has are matched to it, so their rows leave the list and stats count them as that song. Request on a row picks the album with `lidarr_pick` (a song asks for its album) and goes through `lidarr_request_preview` and `lidarr_request` as usual. "Request these" previews each picked album, then sends them with `lidarr_request_batch` after one confirm.

Each `import_overview` report has `history: { plays, playsMatched, songs, songsMatched, firstPlayed, lastPlayed }` for the listening history a source brought in, so a history-only import reads as plays and songs rather than "0 of 0 songs". An import with nothing usable fails with a message naming what was left out.

Settings (synced): `showPlayedALot` (on), `playedMinPlays` (5, from 1 to 1000), `playedPeriod` (`all`).

### Bring in my listening

With `bringIn` on, Platter asks Lidarr for albums from Played a lot in waves while it's open: albums with at least the first wave's plays, most played first, then the next wave, at most `bringInPerDay` a local day. Each album is picked on MusicBrainz and sent the way a request by hand is, full setup check included; in practice mode it's only listed. Albums that can't be picked, or that a check refuses, go to Needs you with the reason. It pauses while Lidarr's queue holds more than `bringInMaxQueue`, while less than `bringInMinFreeGb` is free on the root folder Lidarr downloads into, while Lidarr can't be reached or its check is red, and after three failed requests in a row, and carries on once that clears. Progress is kept on the device. Requests, pauses and practice runs go to the Changes log, and a notification sums up each day. With a Platter Bridge paired, the desktop hands the waves to it (the albums still to ask for, the settings and what was already asked for) and stops running them itself; the Bridge's progress comes back over the sealed link, and `bridge` names the Bridge while it has them. Turning it off stops the Bridge; a Bridge silent for over an hour gives them back.

| Command | Args | Returns |
| --- | --- | --- |
| `bring_in_status` | – | `BringInStatus` |
| `bring_in_pause` | `{ paused }` | `BringInStatus`. Pauses it, or carries on |
| `bring_in_needs_you` | `{ key, retry }` | `BringInStatus`. Takes an album off Needs you; with `retry` the next pass looks at it again (a "Get albums" album goes back on its queue) |
| `get_albums_unqueue` | `{ releaseGroupIds: string[] }` | `BringInStatus`. Takes albums off the "Get albums" queue, all of them when the list is empty |

`BringInStatus = { on, practice, ready, waves, wave, threshold, done, remaining, today, perDay, paused, userPaused, finished, requestedTotal, needsYou: { key, artist, album, plays, reason, at }[], practiced: { key, artist, album, releaseGroupId, at }[], queued, bridge: string | null }`.

Settings (synced): `bringIn` (off), `bringInWaves` (`[100, 50, 20]`, 1 to 5 different counts), `bringInPerDay` (30), `bringInMinFreeGb` (50), `bringInMaxQueue` (20).

## Physical collection

Albums the user owns on vinyl, CD, cassette or another format, with notes on each copy. Marks live in the sync store's `physical` namespace (keyed by album id, synced with the library group). The optional Discogs import reads the user's own collection with their personal token, which is kept in the keychain under `discogs-token` and never returned, logged or put in a URL. Discogs is asked at most once a second; a 429 pauses it.

| Command | Args | Returns |
| --- | --- | --- |
| `physical_list` | – | `OwnedAlbum[]` by artist then title |
| `physical_set` | `{ albumId, owned: { copies, title, artist, thumb } }` | `OwnedAlbum`, or null when `copies` is empty (the mark comes off) |
| `physical_stats` | – | `ShelfStats` |
| `physical_find_release` | `{ artist, title }` | the `CatAlbum` MusicBrainz has for a record on the shelf, or null |
| `discogs_status` | – | `{ saved, username, importing, report }` |
| `discogs_token_set` | `{ token }` | the status; Discogs is asked whose token it is before it's saved (`invalid` when refused). null removes it and the last report |
| `discogs_import` | – | `DiscogsReport` |

`OwnedCopy = { format: "vinyl"|"cd"|"cassette"|"other", pressing, colour, condition, notes, discogs?: { releaseId, instanceId } }`, at most 20 per album, each note up to 300 characters.
`OwnedAlbum = { albumId, title, artist, thumb, copies, formats, at }`; `formats` lists each format once in that order.
`ShelfStats = { albums, copies, byFormat: { format, albums, copies }[], mostPlayed: ShelfAlbum[], missing: ShelfAlbum[] | null }`, counting only albums the library has. `missing` is the albums played most that aren't owned (not unreleased ones), null while `shelfMissingList` is off.
`DiscogsReport = { username, importedAt, total, matched, added, unmatched: { releaseId, artist, title, year, format, thumb, links }[] }`.

An import matches each collection item to a library album: a Discogs release already linked to an album stays with it, then the folded artist and title (`identity::fold`, Discogs' " (2)" and "*" taken off, the first credited artist when the whole credit doesn't match). Unreleased albums never match. Copies added by hand stay, a Discogs copy keeps the user's edits, and copies of records no longer in the collection go. Marks on albums the library doesn't list right now are left alone, and an empty library refuses to import. `physical://changed` fires after any change; `syncstore://changed` names `physical` when another device changed the marks. Settings: `showPhysicalCollection` (on) and `shelfMissingList` (off), both synced.

## Sleeve notes

An album's liner notes from every source Platter reads, each fact with where it came from. The album page asks for them when its Sleeve notes tab opens; a song's Credits item asks for one song.

| Command | Args | Returns |
| --- | --- | --- |
| `sleeve_notes` | `{ id }` (album) | `SleeveNotes` |
| `song_credits` | `{ id }` (song) | `SongCredits` |
| `credit_page` | `{ name }` (a person's name, any case) | `CreditPage`, or null when nothing in the library credits them |

`SleeveNotes = { albumId, looking, lookups: { online, musicbrainz, discogs, unreleased }, released?, editionDate?, recorded?, label?, catalogue?, country?, barcode?, format?, edition?, status?, origin?, notes?, studios: { name, role: "recorded"|"mixed"|"mastered", sources }[], editionNotes: Fact[], groups: RoleGroup[], tracks: { trackId, title, disc, number, groups: RoleGroup[], recorded? }[], links: { source, url }[] }`
with `Fact = { value, sources: Source[] }`, `Source = "tags"|"plex"|"musicbrainz"|"discogs"`, `RoleGroup = { role, people: { name, key, artistId?, details: string[], sources: Source[] }[] }`.
`SongCredits = { trackId, albumId, looking, lookups, song?, links }`, `song` being that song's entry in `tracks`.
`CreditPage = { key, name, artistId?, groups: { role, albums: { album: Album, whole, songs: { id, title }[], details: string[], sources: Source[] }[] }[] }`.

Roles, in the order groups come: `producer`, `writer`, `composer`, `lyricist`, `arranger`, `performer` (with the instruments in `details`), `featured`, `conductor`, `engineer`, `mixer`, `mastering`. Album `groups` hold everyone on the album and its songs except featured artists, who stay with their songs. Dates are "1997", "1997-05", "1997-05-21" or a span "1996-11/1997-02". `status` is the files' release status (`released`, `unreleased`, `leaked`, `alternate`), `origin` their `ARCHIVE_SOURCE`, and `editionNotes` say how the album differs from the library's other editions (albums sharing its `ARCHIVE_GROUP`) by their songs. `key` is the person's folded name; `artistId` is a library artist with that name.

Sources and precedence: the files' own tags (credit tags such as `PRODUCER`, `COMPOSER`, `LYRICIST`, `ENGINEER`, `MIXER`, `PERFORMER` "Name (instrument)", ID3 `TCOM`, `TEXT`, `TIPL` and `TMCL`, and `LABEL`, `CATALOGNUMBER`, `BARCODE`, `RELEASECOUNTRY`, `ORIGINALDATE`, read by the file tag scan), then Plex (the album's release date and label, and the composer, writer, lyricist and producer Plex keeps on each song), then MusicBrainz (the release and its recordings' relations, studios and annotation), then Discogs (the release's credits, label, format and notes). Values that agree add their source to the fact; a date that agrees keeps its most precise form. When values disagree the earlier source wins.

`sleeve_notes` answers from the cache at once and starts one background refresh for the album when something is missing or stale; `looking` is then true and `sleeve://changed` fires with `{ id }` when it's done. The refresh asks Plex for the songs' credits (60 songs at most, kept 7 days), then, while `sleeveLookups` is on and the app isn't offline, MusicBrainz through the artist info cache and its one-request-a-second pacing (needs `infoMusicbrainz` on; a release read by an older build is read again) and Discogs when a token is saved (the release of an owned Discogs copy, else a database search by artist and title taking only a result with the same folded artist and title; kept 30 days, a miss 7). Unreleased albums are never looked up online. A failed lookup waits ten minutes before it's tried again. Offline, the cached answers show.

Each time an album's notes are put together, the people the other sources credit are kept for credit pages (`sleeve_credits`); credit tags in files are kept as the files are read (`tag_credits`), so `credit_page` covers the whole library's tags and every album whose notes were opened. Albums with a cached MusicBrainz or Discogs release are added the first time a credit page opens.

Settings: `showSleeveNotes` (on) shows the tab and the Credits item; `sleeveLookups` (on) allows the online lookups. Both synced.

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
`lyrics` (lyrics written in the editor, by track id), `transcripts`
(transcriptions, keyed `<track id>|<model>`), `lyricsShared` (songs whose
lyrics were shared to LRCLIB) and `smartPlaylists`
(Platter smart playlists, synced with the library folders). Without a sync file this is where the state is kept.

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
| `search` | `{ query, filter: "all"|"songs"|"artists"|"albums"|"playlists", limit? }` | `SearchResults { top?: SearchHit, songs: Track[], artists: Artist[], albums: Album[], playlists: Playlist[], tookMs, source: "local"|"server", corrected?: string }` |
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

Matching is forgiving (`db/searchtext.rs` holds the rules and the table of
spellings): case, accents, apostrophes and most punctuation fold away, "$"
inside a word reads as "s", "&" and "+" as "and", and shorthand matches its
long form both ways (thru/through, tho/though, nite/night, tonite/tonight,
lite/light, u/you, ur/your, n/and, w/with, pt/part, vol/volume,
ft/feat/featuring, no/number, 1 to 10 as digits, words and roman numerals
II to X). Words match in any order, each as a prefix; "and" and small words
like "the" between others are not required. Titles split on " / ", " - "
and "+" rank a match on one part like a title match.

When the words as typed find fewer than five results, words of five or more
letters also match library words one letter off (wrong, missing, extra or
swapped), ranked below the rest; `corrected` then holds the query with the
fix, which the search box offers as its first suggestion. Setting
`searchTypos` turns this off. When nothing matches every word, words the
library has nowhere are left out (else one word at a time), ranked lowest.

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

## Signal path
| Command | Args |
| --- | --- |
| `signal_path` | – → `SignalPath` |

What happens to the sound between the file and the output device, read only.
The engine notes the source format when a song's decoder opens and the device
format when the output opens; the snapshot is built from those and from what
the player already holds. The output thread does no extra work for it.

```
SignalPath = {
  trackId: string | null,
  source: { codec, lossless, bitDepth, sampleRate, channels, bitrate, transcoded, from: "server" | "cache" | "download" } | null,
  decode: string | null,              // "32-bit float"
  loudness: { on, mode: "smart" | "track" | "album" | "off", gainDb: number | null },
  eq: { eq, correction, correctionPreampDb, preampDb, limiter },
  resample: { fromRate, toRate, fromChannels, toChannels, quality } | null,
  blend: { automix, crossfade: "smart" | "fixed" | "off", crossfadeMs },
  output: { device, mode: "shared" | "exclusive" | "fallback", reason, rate, channels, bits, network, volume, volumeBypassed },
  verdict: { kind: "idle" | "bitPerfect" | "lossless" | "lossy", text },
  why: { text, setting: string | null } | null,
}
```

`source.codec` is the format actually decoded, upper case. `transcoded` is
true when the server sent a conversion (a streaming quality cap, or because
the file couldn't be played directly); then `bitDepth` and `bitrate` are null
since the file's own values don't describe the stream. While bit-perfect
output has the device, every sound setting reads as off and `volumeBypassed`
is true. `verdict.text` is plain words: "Bit-perfect", "Lossless, resampled to
48 kHz", "Lossless, volume and EQ applied", "Lossy source (MP3 320)". `why`
says what keeps it from being bit-perfect, and `setting` names the Settings
row that changes it (`bitPerfectOutputs`, `qualityHome`), or null when no
setting can.

Events:
- `audio://signal-path` → `SignalPath` (when it changes: a new song, its
  source becoming known, another device, a sound setting, volume)

## Listen on
The user's other computers running Platter, Plex players Platter can move
playback to (Plexamp, Plex apps, Sonos linked to Plex), DLNA renderers, Chromecasts and
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

DLNA and UPnP renderers are found with an SSDP search for
`urn:schemas-upnp-org:device:MediaRenderer:1` each time the list is
refreshed. Only answers whose description URL is plain http on the address
that answered are read, and only control URLs on that same host are used.
Their ids are `dlna:<UDN>`, `via` is `dlna` and `product` is the model.
A move asks `GetProtocolInfo` which formats the renderer takes and hands it
one song at a time with `SetAVTransportURI` (with DIDL-Lite title, artist,
album and cover), then `Play` and `Seek` to the position. The URL is the
original file on the server when the renderer takes its format (FLAC, MP3,
AAC, Ogg, WAV), else the server's MP3 transcode; a renderer that refuses the
original (UPnP error) gets the transcode. If it refuses that too, the move
fails with "Can't play on <device>: doesn't play this song's format." and
playback stays here. Platter keeps the queue: when a song plays to its end
the next one is handed over (repeat one and all are followed; shuffle can't
change there). Pause, seek and stop are AVTransport calls; volume is
RenderingControl `SetVolume`. A renderer that stops answering ends the
session like a Plex player ("Couldn't reach <device>"); one that's stopped
midway, or starts playing something else, ends it with `stoppedOn`.

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

## Themes and looks
| Command | Args | Returns |
| --- | --- | --- |
| `themes_list` | – | `ThemeFolder` |
| `themes_import` | – | `ThemeFile`, or null when the file picker was cancelled. The file is checked first and copied into the folder under its name |
| `themes_import_text` | `{ text }` | `ThemeFile`. The text of a file dropped on the window, checked and saved into the folder |
| `themes_save` | `{ look: ThemeFile, replace: bool }` | `ThemeFile` as saved. With `replace` it writes over its own `.platter-look` file; otherwise it takes a free name |
| `themes_export` | `{ look: ThemeFile }` | The file name, or null when the save dialog was cancelled |
| `themes_open_folder` | – | – (shows the folder in Explorer or Finder) |

`ThemeFolder = { folder: string, themes: ThemeFile[], invalid: { file, reason }[] }`.
`ThemeFile = { file, name, base: "dark" | "light", tokens: Record<string, string>, author?, style?, fonts?, layout?, notes? }`.

Theme files are JSON in a `themes` folder in the app's data folder:

```json
{ "name": "Harbour", "base": "dark", "tokens": { "color-bg-base": "#08121A", "radius-card": "12px" } }
```

A theme file has `name`, `base` and `tokens`, and may have `author`. Tokens
start from the base's built-in theme. Only the token names in
`src/design/themes/contract.json` are taken (with or without a leading `--`).
Colour tokens take `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`,
`hsl()` or `hsla()`; the radius tokens take a pixel size like `8px`. Files
over 64 KB, unknown fields or tokens, `url()`, CSS (`;`, braces, comments,
`var()`) and markup are refused, and the file is listed under `invalid` with
the reason. The UI applies the same checks again before using a theme.

A look is a `.platter-look` file: a theme with a format `version` (1 for
now), plus an optional `style`, `fonts` and `layout`. The folder lists both
kinds.

```json
{
  "version": 1,
  "name": "Dusk",
  "author": "Sam",
  "base": "dark",
  "tokens": { "color-accent": "#AABBCC" },
  "style": { "density": "compact", "shadow": "subtle", "blur": "low", "border": 1 },
  "fonts": { "text": "manrope", "headings": "fraunces" },
  "layout": { "sidebarSide": "right", "playerButtons": ["queue", "volume"], "coverShape": "circle" }
}
```

Every value is a choice from the `look` part of the contract: densities,
shadows, blur levels, a border of 0 to 3 pixels, the bundled fonts, and the
layout options. Layout lists hold the shown items in order; an item left out
is hidden. A missing part keeps its default. In a look, names Platter doesn't
know are left out and listed in `notes`; a value that isn't allowed refuses
the whole file with the reason, as does a newer format version. Export and
save write only the look's own fields, so nothing else from this computer
goes into a shared file. A look held in the `themeFile` setting must have
nothing left out.

## Settings and tracking
| Command | Args |
| --- | --- |
| `settings_get` | – → `Settings { privateSession, reportPlays, volume, shuffleStyle, ... }` |
| `settings_set` | `{ patch: Partial<Settings> }` |
| `art_cache_info` | – → `{ usedBytes, capBytes }` |
| `art_cache_clear` | – → bytes freed (number) |
| `settings_defaults` | – → `Settings` as a new install has them (Reset this section) |
| `settings_export` | – → path written, or null when the save dialog was cancelled |
| `settings_import` | – → `{ applied, skipped: string[] }`, or null when cancelled |
| `help_open_link` | `{ url }`: opens a help page's link in the browser (project pages only) |

`settings_export` writes `{ kind: "platter-settings", version: 1, settings }`
with every setting a patch can set, except folders, devices and window
places (`NOT_EXPORTED` in settings.rs). No setting holds a key or token, so
the file never does. `settings_import` applies a file as one patch and skips
values this version can't read. `showSetupGuides` and `showHelpButtons`
(both default true, synced) offer setup guides and show the "?" help buttons.

`privateSession` (default false) stops plays being written to the local
listen log. `reportPlays` (default true) controls now-playing and finished
play reports to each song's server; when it's off nothing is sent or queued.
Earlier builds called it `reportToPlex`; that name is still read, and the
sync file keeps it so older builds share the setting. `volume` is the same 0..1
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
| `signalPathBadge` | bool, true | Shows the badge in the player bar that opens the signal path. |
| `resumeContexts` | bool, true | Albums and playlists start where they were left. Off starts at track 1; points are still saved. |
| `precacheCount` | 1 to 50, 5 | Upcoming tracks kept on disk. Out of range values are clamped. |
| `popularSource` | `"global"`, `"myPlays"` | Ranks the artist page's Popular by the global top list (ListenBrainz or Last.fm) or by your own plays. |
| `artCacheMb` | 128 to 51200, 2048 | Artwork cache size. Shrinking it removes the oldest images at once. |
| `artworkColours` | bool, true | Headers tint from artwork. Off gives every header the same neutral colour. |
| `reduceMotion` | `"system"`, `"on"`, `"off"` | Follow the OS setting, or force reduced motion on or off. |
| `theme` | `"studio"` (default), `"studioBlack"`, `"sleeve"`, `"dark"`, `"light"`, `"system"`, `"midnight"`, `"forest"`, `"sand"` | The theme. Studio, Studio Black and Sleeve are Platter's own looks; `dark` is shown as Classic and `light` as Classic light. System follows the computer's light or dark mode between those two. A theme file in `themeFile` takes its place. Settings saved before Studio existed and without a theme read as `"dark"`. See Themes and looks. |
| `accent` | string, `"brass"` | Accent colour: `ivory`, `brass`, `sage`, `sky`, `lilac`, `amber`, `rose`, `teal`, or a custom colour as `#RRGGBB` (stored in capitals). Anything else is refused. Brass is the Studio looks' own accent and ivory the other themes' own. Settings saved before Studio existed and without an accent read as `"ivory"`. |
| `followSystem` | bool, true | Shows the theme's light or dark partner when the computer's mode doesn't match it: Studio and Studio Black pair with Sleeve, the other themes with Classic or Classic light. False for settings saved before it existed. |
| `newLookOffered` | bool, true | The one-time offer to try Studio was shown, or wasn't needed. False for settings saved before Studio existed. This device only. |
| `recordMotion` | `"full"` (default), `"both"`, `"off"` | The record player in now playing: the spinning record and tonearm in the expanded view, there and on the side panel's cover, or neither. `"off"` for settings saved before it existed. |
| `tonearmSeek` | bool, true | The player bar's seek bar is drawn as a stylus on a groove line. It is the same slider either way. False for settings saved before it existed. |
| `themeFile` | `ThemeFile` or null | The theme or look file in use, held whole so it travels in the sync file. Checked like a file in the folder; a bad one is refused. Null clears it. |
| `layout` | `Layout`, Classic | Layout options: `sidebarSide`, `sidebarCollapsed`, `sidebarSections`, `nowPlaying`, `playerStyle`, `playerButtons`, `homeShelves`, `homeGreeting`, `quickAccess`, `headerStyle`, `trackColumns`, `coverShape`, `spinRecord`, `motion`. Checked like a look's layout; a bad one is refused, and a hand-edited bad one goes back to Classic. See Themes and looks. |
| `showLookBuilder` | bool, true | Appearance shows Make your own. |
| `lookStepDone` | bool, false | The first-run Pick a look step was shown or skipped. This device only. |
| `outputDevice` | device id or null, null | Read only here. Set with `output_select`; null follows the system default. |
| `homeShortcuts` | bool, true | Home shows the grid of recent and most played shortcuts. |
| `homeColourWash` | bool, true | Home's top takes its colour from the shortcuts' artwork. Only applies while `artworkColours` is on. |
| `searchSuggestions` | bool, true | Show completions and direct results under the search box while typing. |
| `searchTypos` | bool, true | Search also shows matches one letter off when the words as typed find little ("Forgive typos in search"). |
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
| `infoWikipedia` | bool, false | Artist info from Wikipedia and Wikidata. |
| `infoLastfm` | bool, false | Artist info from Last.fm. Only used while a key is saved and wasn't refused. |
| `infoFanart` | bool, false | Artist images from Fanart.tv. Same key rule. |
| `infoAudiodb` | bool, false | Artist photo and bio from TheAudioDB. Same key rule. |
| `infoListenbrainz` | bool, false | Popular's global list from ListenBrainz (Last.fm top tracks fill in when it has none and Last.fm is on). With this and `infoLastfm` both off, Popular ranks by your plays. The optional token is kept in the keychain. |
| `infoMusicbrainz` | bool, false | MusicBrainz: album release dates and credits, artist members with their years, related artists. Synced. |
| `infoListenbrainzStats` | bool, false | ListenBrainz listener numbers when Last.fm has none. Synced. |
| `linkFeatured` | bool, true | Every name in a track's artist line links to a page, featured artists without a library entry included, and search lists them. Off leaves those names as plain text. |
| `likedMeans` | `"five"` or `"fourUp"`, `"five"` | Which Plex ratings count as liked: five stars (10), or four stars and up (8 or more). Liking always writes five stars. |
| `likeControlRows` | `"hover"`, `"always"` or `"off"`, `"hover"` | When song rows show the like control. Liked songs always show their tick unless it's `"off"`; the player bar and now playing always show it. |
| `showPlexPlayers` | bool, true | List Plex players under Listen on in the device picker. Off makes `remote_players` return an empty list. |
| `moveStopsOldDevice` | bool, true | Moving playback to a player pauses it here, and coming back stops the player. Off leaves both playing. |
| `actAsPlexPlayer` | bool, false | Platter answers as a Plex player so other Plex apps can play here. Opens a local port. This device only. |
| `showCastDevices` | bool, true | List Chromecasts, Cast groups and AirPlay speakers under Listen on. Off stops mDNS browsing. |
| `showDlnaDevices` | bool, true | List DLNA / UPnP renderers under Listen on (`via: "dlna"`). Off stops SSDP searches. |
| `crossfade` | `"smart"`, `"fixed"`, `"off"`, `"smart"` | How one song moves into the next. See Sound below. |
| `crossfadeSeconds` | 1 to 12, 6 | Length of a fixed crossfade. |
| `crossfadeSkipAlbums` | bool, true | Songs of one album played in album order never crossfade. |
| `automix` | bool, true | DJ-style transitions where Automix applies. See Automix below. |
| `automixWhere` | `"playlistsAndMixes"` or `"allButAlbums"`, `"playlistsAndMixes"` | Playlists with Mix on, mixes, radio and what plays after a list ends; or everything but albums. |
| `automixStyle` | `"smooth"` or `"energetic"`, `"smooth"` | Longer, gentler blends, or shorter ones that swap on the bar. |
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
| `sharePublicLinks` | bool, false | Share menus find public links for released songs and albums (iTunes Search and song.link). Off leaves only Platter links and sends nothing. |
| `discordStatus` | bool, false | Show what's playing on Discord. This device only. See Discord above. |
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
| `transcribeModel` | `"small"`, `"medium"`, `"turbo"` or null, null | The speech model; null is Turbo while a graphics card is used and Small without. Medium is a little more accurate than Small, much slower, and works the processor much harder. Turbo is the most accurate and is made for the graphics card. This device only. |
| `transcribeGpu` | `"auto"` or `"off"`, `"auto"` | Transcription and the lyrics check run on the graphics card when there's a usable one (see Lyrics transcription). This device only. |
| `transcribeProcessor` | `"light"`, `"balanced"` or `"fast"`, `"light"` | Processor threads for transcription and the lyrics check (see Lyrics transcription). This device only. |
| `transcribePauseBusy` | bool, true | Albums, the batch and the lyrics check wait while the computer is busy (`paused: busy`). This device only. |
| `transcribeFasterAway` | bool, true | Background speech work may use the `balanced` thread count while nobody is at the computer and it's plugged in. This device only. |
| `showAutoLyrics` | bool, true | `lyrics_get` returns transcriptions the user hasn't edited. Off hides them; they stay saved. |
| `lyricsCheckAudio` | bool, true | Check lyrics against the song with the speech model, once it's installed. Off shows lyrics as not checked. This device only. |
| `lyricsFixTiming` | bool, true | Checked lyrics are moved or stretched to the timing measured in the song. This device only. |
| `lyricsReleasedForVersions` | bool, false | Other versions (unreleased, alternate, live, version tag) take lyrics found for the released version without asking. |
| `lyricsOfficialFirst` | bool, true | Before a song is transcribed, its synced lyrics from every source are checked against it, and a match is shown instead. Transcribed songs are looked at again weekly. |
| `fullWindowHideControls` | bool, true | The expanded view fades its header after 3 s without mouse movement or a key. |
| `readFileTags` | bool, true | Read explicit, version and edition tags from the files in the background. Off stops the scan; tags already read stay. This device only. See File tags above. |
| `musicFolder` | string, `""` | Where this computer sees the server's music files. Empty: tag details are view only. Set with `tagedit_pick_music_folder` or `tagedit_set_music_folder`, not `settings_set`. This device only. |
| `musicFolderServer` | string, `""` | The start of Plex's file paths that stands for the music folder, found when it was set. This device only. |
| `rescanAfterTagEdit` | bool, true | After tags are saved, Plex rescans just the folders written to. |
| `askBeforeManyFiles` | bool, true | A tag save that changes more than 50 files asks first (in the UI). |
| `explicitContent` | `"allowed"` or `"filtered"`, `"allowed"` | Filtered leaves songs tagged explicit out of every context that plays, with one toast per context. |
| `showFanEdits` | bool, false | Off leaves fan and AI edits (`ARCHIVE_FLAGS`) out of album pages and what plays, and hides albums made only of them. |
| `releaseStatusTag` | string, `""` | Where a file's release status is read from: `""` (Auto) reads `ARCHIVE_VERSION`, then `STATUS`, then Picard's release status and type; a tag name (up to 64 characters, no `=`; "auto" counts as Auto) reads that tag after `ARCHIVE_VERSION` instead. A new name reads the files again in the background. Synced. See File tags. |
| `releaseStatusValues` | `{ value, status: "released"\|"unreleased"\|"leaked"\|"alternate" }[]`, `[]` | Custom values and what they mean, checked before the built-in ones. Up to 50, each up to 60 characters, trimmed; the same value (any case) again replaces the earlier one, and blank values are dropped. Synced. |
| `countUnmatchedUnreleased` | bool, true | Albums Plex couldn't match (guid `local://`, or no release type and no label before the next sync) count as unreleased for sharing, Discord, scrobbling, mixes, smart playlists and lyrics transcription until a release type, `ARCHIVE_VERSION` or a release status tag (`releaseStatusTag`) says otherwise. Off: only demo release types, archive version tags and release status tags count. |
| `checkUnmatchedMusicbrainz` | bool, false | Look albums Plex couldn't match up on MusicBrainz in the background (needs `infoMusicbrainz`). A sure match means Plex missed it and the album counts as released. Off, or with MusicBrainz off, stored answers stop counting. Synced. |
| `groupEditions` | bool, true | Albums sharing `ARCHIVE_GROUP` list as one album (the face) in the library, artist pages and Home's album shelves. |
| `plexEditShow` | bool, true | Edit in Plex on album and artist menus, for servers the account owns. Off refuses every edit. |
| `plexEditLock` | bool, true | Fields changed with `plex_edit_details` are locked in Plex so a refresh keeps them. |
| `plexEditMatchUnreleased` | bool, true | Fix match is offered on unreleased albums (always with a warning). Off refuses it there. |
| `dragAndDrop` | bool, true | Songs, albums and playlists can be dragged onto playlists, Liked Songs and the queue. Off stops those drags; reordering inside a playlist or the queue still works. |
| `showToMyDevices` | bool, false | Announce this computer to the user's other Platter computers, open the control port and publish its addresses to plex.tv. Off withdraws the announcement and closes the port. Independent of `actAsPlexPlayer`. This device only. |
| `computerName` | string, `""` | The name other devices see, up to 60 characters, trimmed. Empty uses the OS computer name. This device only. |
| `showAirplayLocked` | bool, false | List AirPlay receivers that need a password or a pairing code. Computers' own receivers are never listed. This device only. |
| `downloadQuality` | `"original"`, `"320"`, `"192"` or `"128"`, `"original"` | Original keeps the file as it is; the others are MP3s from the server's transcoder. Applies to new downloads. This device only. |
| `downloadLimitGb` | number, 20 | Downloads stop at this size, 1 to 4096. This device only. |
| `downloadFolder` | string, `""` | Where downloads go; empty is the app data folder. Changed with `downloads_choose_folder`, not `settings_set`. This device only. |
| `downloadHomeOnly` | bool, true | Downloads wait while the server is reached over remote access or the relay. This device only. |
| `downloadLiked` | bool, false | Keep Liked Songs downloaded; new likes download too. The same as the Liked Songs download toggle. This device only. |
| `deviceFormat` | `"original"`, `"flac"`, `"mp3-320"`, `"mp3-v0"` or `"opus"`, `"original"` | What songs become on a device that doesn't pick its own format. Synced. |
| `deviceDeleteRemoved` | bool, false | Syncing a device removes Platter's own files for songs no longer chosen. Synced. |
| `homeMixes` | bool, true | Home shows Made for you, Your mixes by mood and Your genre mixes. |
| `homeOnThisDay` | bool, true | Home shows On this day: what you played on today's date in earlier years. Synced. |
| `homeTimeTravel` | bool, true | Home shows Time travel: a mix per decade and your year. Synced. |
| `homeRecordsFirst` | bool, true | In Studio, Studio Black and Sleeve, Home leads with records (`home_records`): the album you were in the middle of, then New arrivals, Spun this week, Finish what you started and Not played in a year, before the usual shelves. Other looks keep the usual Home. Synced. |
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
| `shareActivity` | bool, false | Send Plex now-playing timelines, so friends on the server see you live. Off (and in a private session) none go out; finished plays still reach Plex's history. |
| `whatsNewAnnouncements` | bool, true | The bell's dot for unseen What's New items. |
| `notifyNewReleases` | bool, false | A desktop notification when a followed artist gets new music in the library. This device only. |
| `listenGuestsControl` | bool, true | "Let others change what's playing": guests in a session you host can play, pause, skip, seek and reorder. Off, they can only add songs. |
| `listenInviteHours` | 1, 24 or 168, 24 | How long an invite works (each works once). |
| `listenName` | string, `""` | The name others in a listening session see; empty is your Plex name. 40 characters at most. |
| `listenRelay` | string, `""` | The listen along relay, an ntfy server URL; empty is ntfy.sh. A URL of your own goes in your invites. |
| `listenHideUnreleased` | bool, false | In a session you host, unreleased songs go to guests without their title and artist. Synced. |
| `shareOfferServer` | bool, true | Offer "Let them listen from my server" to a guest who can't hear the song (host only). Synced. |
| `unreleasedSharing` | `never`, `ask` or `always`, `ask` | Share unreleased music. Choosing it in Settings sets the unreleased switches for Discord, mixes, scrobbling, listening sessions, shared playlists and Blends, and public transcriptions, which stay as exceptions. Settings saved before it get the rule their switches make (all share: `always`, none: `never`, else `ask`). Synced. |
| `shareDefaultLibraries` | string[], [] | Libraries I share by default: section ids on your server, music only. None until picked. Synced. |
| `shareHideUnreleased` | bool, true | Hide my unreleased music from people I share with (a Plex label the share leaves out; needs Plex Pass). Synced. |
| `shareAllowDownloads` | bool, false | Allow downloads for people I share with. Synced. |
| `shareAcceptInvites` | bool, true | Accept invites inside Platter: a notification and a card, one click to accept. Synced. |
| `discordListenAlong` | bool, true | Discord shows a listening session you host and how many are in it. This device only. |
| `syncEnabled` | bool, false | Keep app-only state in a sync file in `syncFolder`. On with no folder counts as off. This device only. See Sync, folders and keys. |
| `syncFolder` | string, `""` | The folder the sync file goes in. Changed with `sync_file_use_folder` or `sync_file_choose_folder`, not `settings_set`. This device only. |
| `syncSettings` | bool, true | What to sync: settings that aren't this device only. This device only. |
| `syncLibrary` | bool, true | What to sync: library folders and pins. This device only. |
| `syncFollows` | bool, true | What to sync: followed artists, hidden and snoozed items, bookmarks. This device only. |
| `syncLyrics` | bool, true | What to sync: lyrics written in the editor. This device only. |
| `bookmarksInLibrary` | bool, true | A Bookmarks row under Liked Songs in the library. |
| `pinsInLibrary` | bool, true | Pins (up to 4 playlists, albums, artists or folders, `pins_toggle`) sit at the top of the library. |
| `showPronunciation` | bool, false | A romanised line under lyrics in Japanese, Korean, Chinese, Hindi and Cyrillic (`lyrics_pronounce`). |
| `translationService` | string, "" | A LibreTranslate-compatible address for `lyrics_translate`; empty means no translation. This device only. |
| `translateTo` | string, "en" | The language code lyrics are translated into. |
| `libraryDoctor` | bool, true | The library doctor checks the library in the background and shows a count in the app menu. |
| `doctorChecks` | `{ duplicates, covers, albumTags, splitAlbums, albumArtist, lowQuality, unmatched, soundsLike }`, all true | Which library doctor checks run. |
| `doctorCoverPx` | 300, 500, 600 (default), 800, 1000 or 1400 | Covers smaller than this on their longest side count as small. |
| `releaseRadar` | bool, false | The release radar looks for new releases about once a day and shows them on Home and its page. Synced. |
| `radarNotify` | bool, true | A notification when the release radar finds something new (not in a private session). Synced. |
| `autoRequest` | bool, false | Auto-request new albums for the artists it's on for. Turning it on for an artist turns this on. This device only. |
| `autoRequestCap` | 1, 2, 3 (default), 5 or 10 | Most automatic requests a day. This device only. |
| `identifyBySound` | bool, false | Fingerprint songs on this computer and look them up on AcoustID (only the fingerprint and length are sent). This device only. |
| `soundIdScope` | `"untagged"` (default) or `"all"` | Which songs: files with no recording id or ISRC, or every song. This device only. |
| `subsonicCheckMinutes` | 1, 5 (default), 15, 30 or 60 | Minutes between asking each Subsonic server whether its music changed. This device only. |
| `saveQueueOnServer` | bool, false | Keep the queue on servers that hold one, so another app can pick it up. Synced. |
| `scrobbleLastfm` | bool, false | Scrobble to Last.fm once signed in. This device only. |
| `scrobbleListenbrainz` | bool, false | Scrobble to ListenBrainz with the user token. This device only. |
| `scrobbleUnreleased` | bool, false | Unreleased music scrobbles too. |
| `aiProvider` | `"none"` (default), `"anthropic"` | The AI provider for prompted playlists. None hides them. This device only (the key is in this computer's keychain). |
| `aiModel` | string, `"claude-opus-5-5"` | The provider's model. Letters, numbers, dots, dashes; empty goes back to the default. |
| `miniplayerOnTop` | bool, true | The miniplayer window stays above other windows. This device only. |
| `miniplayerTint` | bool, true | The whole miniplayer takes a dark shade of the cover's colour; off, the colour is on a card behind the cover. This device only. |
| `miniplayerQueue` | bool, false | The miniplayer lists the next songs when it's at least 500 tall, or beside the art when it's side by side. This device only. |
| `miniplayerSideBySide` | bool, true | A miniplayer more than 1.4 times as wide as it is tall (and at least 480 wide) puts the title, progress and queue beside the art. This device only. |
| `miniplayerBounds` | `{ x, y, width, height }` or null, read only | Where the miniplayer was last, in logical pixels. Kept by the core. This device only. |
| `powerKeys` | bool, false | Keyboard power mode in the main window: jump hints, j / k, g g, G, Enter and /. Handled in the UI (`src/shell/powerKeys.ts`); the core only stores it. |
| `hintKey` | `"f"` (default), `"h"`, `";"`, `"'"` | The key that shows the jump hints. |
| `nowPlayingFeed` | bool, false | Serve the playing song on 127.0.0.1 for widgets (see Now-playing feed). This device only. |
| `nowPlayingFeedPort` | 1024 to 65535, 32540 | The feed's port. Lower ports are refused with `invalid`. This device only. |
| `allowStreamDeck` | bool, false | Let a paired Stream Deck control playback on 127.0.0.1 (see Stream Deck). This device only. |
| `allowAutomation` | bool, false | Let `platter://` command links, paired scripts and the `platter` command line control playback (see Stream Deck and AUTOMATION.md). This device only. |

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

Automix. Where it applies (`automix`, `automixWhere`, and per playlist
`automix_mix_set`), each transition is planned from both songs' analyses.
When both have a steady beat found with confidence and their tempos are
within 6% (allowing for half and double time), the outgoing song is played
up to 6% faster or slower to the incoming one's tempo and the two meet on a
bar line (on a beat when the bars aren't clear). Otherwise it's an
equal-power fade as long as a fixed crossfade, or 6 s. Calmer songs and the
smooth style blend for longer; a blend never takes more than 30% of the
shorter song or more than 24 s, and clashing keys get a shorter, plain
blend. The incoming song skips its lead-in. There is no blend between songs
of one album in album order, between songs of one album that run into each
other, for spoken word, for songs under 30 s, while repeating one song, on
albums, or when the transition picked is "No transition". The incoming song
takes over (`Ended`, then the next song's start) at the blend's midpoint;
until then the position is the outgoing song's. Next during a blend before
its midpoint goes straight to the incoming song where it is, at full
volume. A seek cancels the blend. On a shuffled mixed playlist the upcoming
order is rearranged so neighbouring songs fit (tempo, key, energy), picking
each next song from the next few in the shuffled order.

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
