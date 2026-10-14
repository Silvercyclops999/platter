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
| `albums` | `{ sort: "name"|"artist"|"year"|"recent"|"plays", offset, limit, section?: ReleaseSection }` | `Page<Album>` |
| `playlists` | – | `Playlist[]` |
| `artist_page` | `{ id }` | `ArtistPage { artist, popular: Track[], sections: { section, albums: Album[] }[] }` |
| `album_page` | `{ id }` | `AlbumPage { album, tracks: Track[], totalDurationMs, moreByArtist: Album[] }` |
| `playlist_page` | `{ id }` | `{ playlist, tracks: Track[] }` |

`Page<T> = { items: T[], total }`.
`SyncStatus = { phase: "idle"|"artists"|"albums"|"tracks"|"incremental", done, total, lastSyncAt? }`
Event `sync://progress` → `SyncStatus`; event `library://changed` (no payload)
after any sync writes, so open pages refetch.

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
| `set_shuffle` | `{ shuffle }` |
| `set_repeat` | `{ repeat: "off"|"all"|"one" }` |
| `queue_add` | `{ trackIds, at: "next"|"end" }` |
| `queue_remove` | `{ itemId }` |
| `queue_move` | `{ itemId, toIndex }` (index within the user queue) |
| `queue_clear` | – (clears the user queue only) |
| `queue_jump` | `{ itemId }` (play that item now) |
| `player_state` | – → `PlayerState` |

`PlayerState = { status: "idle"|"loading"|"playing"|"paused"|"buffering"|"error", track?: Track, positionMs, durationMs, volume, muted, shuffle, repeat, queue: QueueView }`
`QueueView = { current?: QueueItem, userQueue: QueueItem[], nextUp: QueueItem[], history: QueueItem[], context?: { kind, id?, title? } }`
`QueueItem = { itemId, track: Track, source: "user"|"context" }`

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
| `settings_get` | – → `Settings { privateSession, reportToPlex, volume, ... }` |
| `settings_set` | `{ patch: Partial<Settings> }` |

`privateSession` (default false) stops plays being written to the local
listen log. `reportToPlex` (default true) controls timeline and scrobble
reports; when it's off nothing is sent or queued. `volume` is the same 0..1
value as `set_volume`; setting it here also moves the player. Settings are
stored in the local database and apply immediately.

## Updates
| Command | Returns |
| --- | --- |
| `update_check` | `{ available: bool, version?, notes? }` |
| `update_install_on_restart` | – |

Event `update://ready` → `{ version }`.
