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

`AuthStatus = { state: "signedOut" | "pending" | "signedIn", username?, server?: ServerInfo, needsServerPick: bool, reauthRequired: bool }`

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
| `home` | – | `HomeData { recentlyPlayed: Album[], recentlyAdded: Album[], jumpBackIn: Album[] }` |
| `artists` | `{ sort: "name"|"recent"|"plays", offset, limit }` | `Page<Artist>` |
| `artist_names` | – | `{ id, title }[]` (every artist) |
| `albums` | `{ sort: "name"|"artist"|"year"|"recent"|"plays", offset, limit, section?: ReleaseSection }` | `Page<Album>` |
| `playlists` | – | `Playlist[]` |
| `artist_page` | `{ id }` | `ArtistPage { artist, popular: Track[], sections: { section, albums: Album[] }[], plays, about?, similar: Artist[] }` |
| `album_page` | `{ id }` | `AlbumPage { album, tracks: Track[], totalDurationMs, moreByArtist: Album[] }` |
| `playlist_page` | `{ id }` | `{ playlist, tracks: Track[] }` |

`Page<T> = { items: T[], total }`.
`artist_names` feeds the artist links in track lines: the UI loads it once,
keeps it, and loads it again after `library://changed`. A track's
`ratingCount` (global listener count, absent when Plex has none) ranks the
artist page's `popular` list: songs with a count first, highest first, then
the rest by your own plays. `popular` also has songs where the artist is
credited in the track artist line, at most 10, one copy per song.
On the artist page, `plays` is the sum of play counts over the artist's own
tracks, `about` is the Plex artist biography (null when empty), and `similar`
holds the Plex similar artists that are in this library, at most 12, in
Plex's order. Artist lists never carry the biography.
`SyncStatus = { phase: "idle"|"artists"|"albums"|"tracks"|"incremental", done, total, lastSyncAt? }`
Event `sync://progress` → `SyncStatus`; event `library://changed` (no payload)
after any sync writes, so open pages refetch.

## Now playing
| Command | Args | Returns |
| --- | --- | --- |
| `now_playing_details` | `{ trackId }` | `NowPlayingDetails { trackId, artist?, credits, lyrics? }` |

The extras the now-playing panel shows for a track, read from Plex when the
panel needs them (not cached in the library database).
`artist = { id, title, summary?, thumb?, art? }` is the album artist with its
Plex bio and photo. `credits = { name, roles: ("composer"|"writer"|"lyricist"|"producer")[] }[]`
lists people Plex credits beyond the track's artists; it is often empty.
`lyrics = { synced, lines: string[] }` is there only when the track has a
lyrics stream with words in it. Lines come without timestamps and `""` marks a
break between verses; `synced` says the source was timed. Only the track
request can fail the call: when the artist or lyrics request fails, that part
is just left out.

## Search
| Command | Args | Returns |
| --- | --- | --- |
| `search` | `{ query, filter: "all"|"songs"|"artists"|"albums"|"playlists", limit? }` | `SearchResults { top?: SearchHit, songs: Track[], artists: Artist[], albums: Album[], playlists: Playlist[], tookMs, source: "local"|"server" }` |
| `recent_searches` | – | `string[]` |
| `recent_search_add` | `{ query }` | – |
| `recent_searches_clear` | – | – |

`SearchHit = { kind: "artist"|"album"|"track"|"playlist", artist?, album?, track?, playlist? }`

Until the first library sync has finished, `search` asks the server instead
(`/hubs/search`), ranks the answer with the same rules and returns
`source: "server"`. If the server is slow or unreachable it answers from the
cache. After the first sync, search never touches the network.

## Playback and queue
| Command | Args |
| --- | --- |
| `play_context` | `{ context: { kind: "album"|"playlist"|"artist"|"tracks"|"search", id?, trackIds? }, startTrackId?, shuffle? }` |
| `play_toggle`, `play_pause`, `play_resume`, `play_next`, `play_previous` | – |
| `play_seek` | `{ positionMs }` |
| `set_volume` | `{ volume: 0..1 }` (linear slider value; core applies the perceptual curve) |
| `set_muted` | `{ muted }` |
| `set_shuffle` | `{ shuffle }` (for the playing context) |
| `set_context_shuffle` | `{ contextId, shuffle }` (applies now if that context is playing, otherwise the next time it plays) |
| `set_repeat` | `{ repeat: "off"|"all"|"one" }` |
| `queue_add` | `{ trackIds, at: "next"|"end" }` |
| `queue_remove` | `{ itemId }` |
| `queue_move` | `{ itemId, toIndex }` (index within the user queue) |
| `queue_clear` | – (clears the user queue only) |
| `queue_jump` | `{ itemId }` (play that item now) |
| `player_state` | – → `PlayerState` |

`PlayerState = { status: "idle"|"loading"|"playing"|"paused"|"buffering"|"error", track?: Track, positionMs, durationMs, volume, muted, shuffle, shuffledContexts: string[], repeat, queue: QueueView }`
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

Events:
- `player://state` → `PlayerState` (on any state/track/queue change)
- `player://position` → `{ positionMs, durationMs, bufferedMs }` (4×/s while playing)
- `player://error` → `{ trackId?, kind, message }` (e.g. "Skipped: file unavailable")

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
| `popularSource` | `"global"`, `"myPlays"` | Ranks the artist page's Popular by listener counts or by your own plays. |
| `artCacheMb` | 128 to 51200, 2048 | Artwork cache size. Shrinking it removes the oldest images at once. |
| `artworkColours` | bool, true | Headers tint from artwork. Off gives every header the same neutral colour. |
| `reduceMotion` | `"system"`, `"on"`, `"off"` | Follow the OS setting, or force reduced motion on or off. |

Quality changes apply from the next track. Settings saved by an older build
load with these defaults for anything missing.

## Updates
| Command | Returns |
| --- | --- |
| `update_check` | `{ available: bool, version?, notes? }` |
| `update_install_on_restart` | – |
| `app_info` | – → `{ version, licence, repoUrl }` |
| `open_project_page` | – (opens the project page in the default browser) |

Event `update://ready` → `{ version }`.
