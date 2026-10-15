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
the others; a track's `ratingCount` only breaks ties there. When there is no
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
| `artist_info_keys` | – | `{ lastfm, fanart, audiodb, listenbrainz: { saved, rejected } }` |
| `artist_info_key_set` | `{ source: "lastfm"|"fanart"|"audiodb"|"listenbrainz", key: string|null }` | `{ saved, rejected }` |
| `artist_info_cache_clear` | – | number of artists that had cached info |
| `open_info_link` | `{ url, artistId? }` | – (opens a source, licence or profile page in the default browser) |

`ArtistInfo = { id, bio?, bioSource?: "plex"|"wikipedia"|"lastfm"|"audiodb", origin?, yearsActive?, members: { name, artistId? }[], tags: string[], listeners?, scrobbles?, photo?, banner?, logo?, credits: { source, text, url?, licence?, licenceUrl? }[], gallery: string[], links: { kind, label, url }[], bioLinks: { text, artistId }[] }`

Optional outside sources fill gaps that Plex leaves: Wikipedia and Wikidata
(no key, on by default), Last.fm, Fanart.tv and TheAudioDB (each needs the
user's own key and is off until one is saved). `artist_info` answers from a
local cache at once and fetches missing or month-old entries in the
background, then emits `artistinfo://changed` with `{ id }`. Only the
artist's name or MusicBrainz id is sent. Sources that are off, failing or
whose key was refused are left out, along with their credits.

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

## Sharing
| Command | Args | Returns |
| --- | --- | --- |
| `share_info` | `{ kind: "track"|"album"|"artist"|"playlist", id, atSeconds? }` | `{ appLink, public: "available"|"unreleased"|"off"|"none" }` |
| `share_public_link` | `{ kind, id }` | the public link (string) |
| `deep_link_take` | – | `DeepLinkOpen` or null |

`DeepLinkOpen = { status: "open"|"notInLibrary"|"invalid", kind?, id?, albumId?, atSeconds?, join? }` (`join`: a listening session invite to pass to `listen_join`)

`appLink` is a `platter://` link: `platter://<song|album|artist|playlist>/<id>`,
plus `?t=<seconds>` for a song when `atSeconds` is given, and `s=<tag>`, eight
hex digits of a hash of the server's id (not the id itself), so a link from
another server isn't mistaken for whatever has the same number here. Links
are read leniently: `track` for `song`, `t` as seconds or `m:ss`, a missing
`s`.

`public` says whether `share_public_link` can work: `unreleased` for music
whose album release type is a demo (the one rule lives in
`db::release::is_unreleased`, where archive tags will join it), `off` when the
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
uploaded once to Imgur and its `i.imgur.com` link kept for good; else, with
`discordUploadCatbox` on, the same upload to catbox.moe's anonymous API
(`reqtype=fileupload`, `fileToUpload`) and its `files.catbox.moe` link kept for
good. Only those hosts are ever sent; anything else falls back to the app
icon. A failed upload isn't retried
until the next start or a new client ID. The client ID lives in the keychain;
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
without an ID3 tag in front), ID3v2.2 to 2.4 `TXXX`, MP4 `rtng` and `----`
items, Ogg Vorbis, Opus and FLAC comments. Kept: `ARCHIVE_VERSION`, `_ERA`,
`_QUALITY`, `_DATE`, `_EDITION`, `_GROUP`, `_SOURCE`, `_FLAGS` and explicit
(`ITUNESADVISORY` 1 or 4, `EXPLICIT=1`, M4A `rtng` 1 or 4; 2 means clean).

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
(synced lyrics within 10 s of the track's length first, then the closest
length; synced lyrics further off keep their words without the times). LRCLIB
answers are cached in SQLite (`lyrics_cache`): found 30 days, none 7 days.
A source that's down, busy or unreadable gives `null` and isn't cached. Only
a malformed id is an error (`invalid`).

`Lyrics = { synced, lines: string[], timed?: LyricLine[], credit?, source: "plex"|"file"|"user"|"lrclib" }`.
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
follow it: every 30 seconds (and after each change made here) new songs queue
and removed songs' files are deleted unless another downloaded item still
holds them. A mix has no list in the cache, so `downloads_set` for a mix needs
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
| `listen_start` | – | `ListenView` with the invite link |
| `listen_stop` | – | – (ends the session you host, or leaves the one you're in) |
| `listen_join` | `{ link }` | `ListenView` |
| `listen_add` | `{ trackIds: string[] }` | – |

`ActivityPanel = { state: "ok"|"notAdmin"|"off"|"offline", friends: FriendActivity[] }`

`FriendActivity = { accountId, name, thumb?, trackId, title, artist, albumId?, album, cover?, at (unix s), now: bool }`

`FollowedArtist = { id, title, thumb?, followedAt (unix s) }`

`WhatsNew = { following: bool, items: { album: Album, new: bool }[], unseen }`

`ListenView = { role: "none"|"host"|"guest", sessionId?, hostName?, invite?, listeners: string[], guestsCanAdd, status?: "connecting"|"live"|"hostGone"|"ended" }`

Events: `follows://changed` → `FollowedArtist[]`, `whatsnew://changed` →
`{ unseen }` (0 while What's New announcements are off), and `listen://state`
→ `ListenView` whenever a session starts, someone joins or leaves, or it ends.

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

A listening session runs on the host's Platter control port (the one "Show
this computer to my other devices" opens), so hosting needs that setting on
and a local network. The invite is
`platter://join/<session>?k=<secret>&s=<server tag>&h=<ip:port,...>&n=<host name>`;
opening one (or `listen_join`) joins. Guests sign every request with a key
made from the secret, ask for the host's state about once a second and
follow it: the same song and spot (half the request's round trip added while
playing), pauses and skips. With `listenGuestsAdd` on, guests' `listen_add`
goes to the end of the host's queue. A host that doesn't answer for 10 s is
`hostGone`; one that ended the session is `ended`.

| Route on the control port | Body | Answer |
| --- | --- | --- |
| `POST /platter/v1/session/join` | `{ name }` | `HostState` |
| `GET /platter/v1/session/state` | – | `HostState` |
| `POST /platter/v1/session/add` | `{ trackIds }` | `{}`, or 403 when the host doesn't let guests add |
| `POST /platter/v1/session/leave` | – | `{}` |

`HostState = { sessionId, hostName, trackId?, positionMs, durationMs, playing, upNext: string[] (50 at most), guests: { id, name }[], guestsCanAdd }`.
A session that has ended answers 410; a request not signed with the
session's key answers 401.

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

`KeyKind = "listenbrainz"|"lastfm"|"fanart"|"audiodb"|"imgur"|"catbox"`

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
| `set_smart_shuffle` | `{ on }` (on also turns shuffle on; plain shuffle off ends it) |
| `set_smooth_transitions` | `{ on }` (crossfade for the playing context only; ends when another context starts) |
| `set_repeat` | `{ repeat: "off"|"all"|"one" }` |
| `set_ab_loop` | `{ abLoop: { aMs, bMs } | null }` (null clears it) |
| `queue_add` | `{ trackIds, at: "next"|"end"|{ index } }` (`index` puts them at that spot in the user queue; past the end appends) |
| `queue_remove` | `{ itemId }` |
| `queue_move` | `{ itemId, toIndex }` (index within the user queue) |
| `queue_clear` | – (clears the user queue only) |
| `queue_jump` | `{ itemId }` (play that item now) |
| `player_state` | – → `PlayerState` |

`PlayerState = { status: "idle"|"loading"|"playing"|"paused"|"buffering"|"error", track?: Track, positionMs, durationMs, volume, muted, shuffle, shuffledContexts: string[], repeat, queue: QueueView, abLoop: { aMs, bMs } | null, smartShuffle, smoothTransitions }`
`QueueView = { current?: QueueItem, userQueue: QueueItem[], nextUp: QueueItem[], history: QueueItem[], context?: { kind, id?, title? } }`
`QueueItem = { itemId, track: Track, source: "user"|"context"|"autoplay"|"smart" }` (`autoplay`: similar music after the context ran out; `smart`: put in by smart shuffle)

Shuffle is remembered per album, playlist or artist id; repeat is one global
setting. `shuffle` is the playing context's state and `shuffledContexts` lists
every context id whose shuffle is on, so a page can show its own toggle for a
context that isn't playing. `play_context` without `shuffle` uses that
context's stored value; passing `shuffle` sets it.

Queue rules the core applies:
- Without `startTrackId`, a context starts at its resume point (the track and
  position it was left at) when shuffle is off, at track 1 if it was never
  played, and on a random track when shuffle is on. Resume points and shuffle
  values are saved with the queue, for the 200 most recently used contexts.
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
- Smart shuffle (shuffle on, with the `smartShuffle` setting) puts one similar
  song after every three upcoming songs of an album, playlist, artist or
  Liked Songs (`source: "smart"`), and more when those run out. Turning plain
  shuffle off takes the ones still to come out again.

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

`OutputState = { devices: OutputDevice[], selected: string | null, current: OutputDevice | null }`
`OutputDevice = { id, name, isDefault }`

`id` is the OS's stable device id. `selected` is the chosen device, or null
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
paused and send the new queue to the same player. Local queue edits (add,
remove, move) stay local until the next send.

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
the Plex account token) over the method, path, sender id, receiver id,
time, nonce and the SHA-256 of the body. The receiver refuses (401) a
missing or wrong signature, a time more than 30 s from its clock, and a
nonce it has seen. The token itself is never sent.

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
instead. A refused signature is an `auth` error ("Couldn't reach <name>:
Couldn't confirm it's signed in to your Plex account.").

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
| `sharePublicLinks` | bool, true | Share menus find public links for released songs and albums (iTunes Search and song.link). Off leaves only Platter links and sends nothing. |
| `discordStatus` | bool, true | Show what's playing on Discord. This device only. See Discord above. |
| `discordClearOnPause` | bool, true | Pausing clears the status. Off shows the song as paused, without a progress bar. |
| `discordShowUnreleased` | bool, true | Off shows nothing while an unreleased song plays. |
| `discordUnreleasedArt` | bool, true | Off shows the app icon for unreleased songs; their cover is never looked up or uploaded. |
| `discordFindSong` | bool, true | The Find this song button. Also needs `sharePublicLinks`. |
| `discordCoverArchive` | bool, true | Covers from Cover Art Archive by MusicBrainz release id, or by a MusicBrainz search on album artist and title for released albums. |
| `discordUploadArt` | bool, false | Upload covers Cover Art Archive lacks to Imgur. Only used while a client ID is saved and wasn't refused. |
| `discordUploadCatbox` | bool, false | Upload covers Cover Art Archive lacks to catbox.moe (no key) when Imgur isn't used. Uploads are public and can't be deleted. |
| `discordStatusLine` | `"app"\|"artist"\|"song"`, `"app"` | What the member list shows after "Listening to" (`status_display_type`). |
| `showPlexAvatar` | bool, true | The app menu button shows the Plex profile picture (`AuthStatus.avatar`). Off shows the first letter of the username. This device only. |
| `lyricsMode` | `"autoShow"\|"off"`, `"autoShow"` | Auto-show opens the expanded view in lyrics mode when a song with synced lyrics starts while now playing is open. Off shows lyrics only from the lyrics button. |
| `lyricsLrclib` | bool, false | `lyrics_get` asks LRCLIB when Plex has no lyrics (sends artist, title, album and length; the search sends artist and title). Off sends nothing. |
| `lyricsPreferOwn` | bool, false | Lyrics written in the app show ahead of Plex's for every song that has them. Off shows them only when Plex has none, or for songs saved with `prefer`. |
| `lyricsShowAdd` | bool, true | The expanded view's lyrics mode offers Add lyrics for a song with none. The track menu keeps Add lyrics either way. |
| `transcribeLyrics` | bool, false | Transcribe songs without lyrics on this computer (Transcribe this song, the album menu, the unreleased batch). See Lyrics transcription. This device only. |
| `transcribeModel` | `"small"` or `"medium"`, `"small"` | The speech model. Medium is more accurate and about three times slower. This device only. |
| `showAutoLyrics` | bool, true | `lyrics_get` returns transcriptions the user hasn't edited. Off hides them; they stay saved. |
| `fullWindowHideControls` | bool, true | The expanded view fades its header after 3 s without mouse movement or a key. |
| `readFileTags` | bool, true | Read explicit, version and edition tags from the files in the background. Off stops the scan; tags already read stay. This device only. See File tags above. |
| `explicitContent` | `"allowed"` or `"filtered"`, `"allowed"` | Filtered leaves songs tagged explicit out of every context that plays, with one toast per context. |
| `showFanEdits` | bool, false | Off leaves fan and AI edits (`ARCHIVE_FLAGS`) out of album pages and what plays, and hides albums made only of them. |
| `groupEditions` | bool, true | Albums sharing `ARCHIVE_GROUP` list as one album (the face) in the library, artist pages and Home's album shelves. |
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
| `smartShuffle` | bool, false | The shuffle button has a third state, smart shuffle. Off ends smart shuffle that's on. |
| `goOffline` | bool, false | No network use: no requests to the server or plex.tv, no artist info, LRCLIB, share link lookups, Discord status, Listen on discovery or update checks. Health shows `offline` with the `goneOffline` banner and `cachedOnly`, so only music on this computer plays. Off reconnects as at startup. This device only. |
| `showFriendActivity` | bool, true | The top bar's friend activity button and panel. |
| `shareActivity` | bool, true | Send Plex now-playing timelines, so friends on the server see you live. Off (and in a private session) none go out; finished plays still reach Plex's history. |
| `whatsNewAnnouncements` | bool, true | The bell's dot for unseen What's New items. |
| `notifyNewReleases` | bool, false | A desktop notification when a followed artist gets new music in the library. This device only. |
| `listenGuestsAdd` | bool, true | Guests in a listening session you host can add songs to your queue. |
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

Quality changes apply from the next track. Settings saved by an older build
load with these defaults for anything missing.

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

Everything runs in the core between decode and output, so it works with the
window hidden. The chain is leveling gain, crossfade mix, EQ, limiter,
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

When the output device changes (a switch, a new system default, an unplug),
the core switches to the preset or profile bound to the new device, or back
to the earlier choice on a device without one, saves it and sends
`settings://changed`.

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
