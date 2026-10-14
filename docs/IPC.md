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
| `artist_page` | `{ id }` | `ArtistPage { artist, popular: Track[], sections: { section, albums: Album[] }[], plays, about?, similar: Artist[], appearsOn: Album[] }` |
| `album_page` | `{ id }` | `AlbumPage { album, tracks: Track[], totalDurationMs, moreByArtist: Album[] }` |
| `playlist_page` | `{ id }` | `{ playlist, tracks: Track[] }` |

`Page<T> = { items: T[], total }`.
`HomeItem` is `{ kind: "album", album }`, `{ kind: "artist", artist }` or
`{ kind: "playlist", playlist }`. Home rows hold 12 items. Recently played
counts a play started from an artist or a playlist for that artist or
playlist, not for each song's album; Plex history from other devices counts
for the album. `shortcuts` holds up to 8: the first 6 recently played, then
the most played album, artist and playlist in turn, then more recent ones,
with no repeats. `playlists` puts the ones played on this computer first,
newest first, then the rest by title. `topArtists` is artists by play count,
played ones only.
`artist_names` feeds the artist links in track lines: the UI loads it once,
keeps it, and loads it again after `library://changed` (and shortly after
`artistinfo://changed`, which can teach new names). It ends with names that
contain a separator, like "Tyler, The Maker", learned from ListenBrainz
artist credits, so a split keeps them whole; their ids are `credit:` ids.

`popular` holds the artist's own songs and songs where the artist is
credited as a whole name in the track artist line, at most 10, one copy per
song. With `popularSource` "global", songs on the artist's global top list
(see Artist info) come first in list order, matched by title folded for
comparison (case, brackets, "feat.", version suffixes like "2011 Remaster" and
punctuation ignored). The rest follow by your own plays, then most recent
play; a track's `ratingCount` only breaks ties there. Until the list has
arrived, or when there is none, the whole list is in plays order. `appearsOn`
lists albums by others whose tracks credit the artist, newest first.

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
| `open_info_link` | `{ url }` | – (opens a source or licence page in the default browser) |

`ArtistInfo = { id, bio?, bioSource?: "plex"|"wikipedia"|"lastfm"|"audiodb", origin?, yearsActive?, members: { name, artistId? }[], tags: string[], listeners?, scrobbles?, photo?, banner?, logo?, credits: { source, text, url?, licence?, licenceUrl? }[] }`

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

The same background fetch gets the artist's global top list for Popular
while `popularSource` is "global": ListenBrainz top recordings by the
artist's MusicBrainz id (no key; an optional user token, sent as
`Authorization: Token`, lets it answer artists it hasn't cached), and when
ListenBrainz has nothing, Last.fm top tracks if Last.fm is on with a key.
Copies of a song are merged and the top 100 are kept for 7 days, "none found"
too. Offline, busy or a bad answer stores nothing and is tried again on a
later page open. A new list emits `artistinfo://changed`, and the artist page
then asks `artist_page` again.

`artist_info` also takes `credit:` ids. Their name comes from the tracks that
credit it, and their MusicBrainz id from a MusicBrainz artist search, taking
only a result with score 100 whose name or alias equals the credited name;
Plex is never asked about them. The other sources then work as for any
artist, cached under the `credit:` id.

Keys are kept in the OS keychain, never in settings. Saving a key checks it
with one lookup; `rejected` means the service refused it, which turns that
source off until another key is saved or the app restarts. `open_info_link`
only opens https links on the sources' and Creative Commons' sites.

## Sharing
| Command | Args | Returns |
| --- | --- | --- |
| `share_info` | `{ kind: "track"|"album"|"artist"|"playlist", id, atSeconds? }` | `{ appLink, public: "available"|"unreleased"|"off"|"none" }` |
| `share_public_link` | `{ kind, id }` | the public link (string) |
| `deep_link_take` | – | `DeepLinkOpen` or null |

`DeepLinkOpen = { status: "open"|"notInLibrary"|"invalid", kind?, id?, albumId?, atSeconds? }`

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
| `play_context` | `{ context: { kind: "album"|"playlist"|"artist"|"tracks"|"search"|"liked", id?, trackIds? }, startTrackId?, shuffle? }` |
| `play_toggle`, `play_pause`, `play_resume`, `play_next`, `play_previous` | – |
| `play_seek` | `{ positionMs }` |
| `set_volume` | `{ volume: 0..1 }` (linear slider value; core applies the perceptual curve) |
| `set_muted` | `{ muted }` |
| `set_shuffle` | `{ shuffle }` (for the playing context) |
| `set_context_shuffle` | `{ contextId, shuffle }` (applies now if that context is playing, otherwise the next time it plays) |
| `set_repeat` | `{ repeat: "off"|"all"|"one" }` |
| `set_ab_loop` | `{ abLoop: { aMs, bMs } | null }` (null clears it) |
| `queue_add` | `{ trackIds, at: "next"|"end"|{ index } }` (`index` puts them at that spot in the user queue; past the end appends) |
| `queue_remove` | `{ itemId }` |
| `queue_move` | `{ itemId, toIndex }` (index within the user queue) |
| `queue_clear` | – (clears the user queue only) |
| `queue_jump` | `{ itemId }` (play that item now) |
| `player_state` | – → `PlayerState` |

`PlayerState = { status: "idle"|"loading"|"playing"|"paused"|"buffering"|"error", track?: Track, positionMs, durationMs, volume, muted, shuffle, shuffledContexts: string[], repeat, queue: QueueView, abLoop: { aMs, bMs } | null }`
`QueueView = { current?: QueueItem, userQueue: QueueItem[], nextUp: QueueItem[], history: QueueItem[], context?: { kind, id?, title? } }`
`QueueItem = { itemId, track: Track, source: "user"|"context" }`

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
- When the context runs out with repeat off, the player goes back to its first
  track, paused at 0:00, with the rest of the context as upcoming.

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

`Health = { status: "local"|"remote"|"relay"|"offline"|"starting"|"updating"|"noAuth", serverName?, connection?, latencyMs?, serverVersion?, lastSeenAt? (Unix ms), scan?: { done, total }, banner?: { kind, message }, filesAvailable: bool, cachedOnly: bool }`

`banner.kind` is one of `offline`, `noInternet`, `starting`, `updating`,
`startTimeout`, `relay`, `signIn`, `accessRemoved`, `filesUnavailable`,
`noMusicLibrary`, `diskFull`; `message` is the text to show. `scan` counts new
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
| `showAirplayLocked` | bool, false | List AirPlay receivers that need a password or a pairing code. Computers' own receivers are never listed. |

Quality changes apply from the next track. Settings saved by an older build
load with these defaults for anything missing.

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
