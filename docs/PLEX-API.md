# Plex API notes

Reference for the Plex client. The tables list the commonly used paths. Check
each one against a real server and write down what you find in the log below
(request, response shape, gotchas). bomer/plexify's `docs/PLEX-API.md`
(https://github.com/bomer/plexify) is also worth reading; it covers every
endpoint that app uses.

Every server request sends:
- `X-Plex-Token` (header)
- `X-Plex-Client-Identifier` (stable per install), `X-Plex-Product`, `X-Plex-Version`,
  `X-Plex-Platform`, `X-Plex-Device-Name`
- `Accept: application/json`

## Account and discovery (plex.tv)
| Purpose | Request | Notes |
| --- | --- | --- |
| Start PIN sign-in | `POST https://plex.tv/api/v2/pins?strong=true` | Returns pin id + code; open the auth URL in the browser |
| Poll for token | `GET https://plex.tv/api/v2/pins/{id}` | `authToken` appears once approved |
| List servers | `GET https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1` | Each server lists connections: local, remote, relay. Try local first |

## Server health
| Purpose | Request |
| --- | --- |
| Identity / is it up | `GET /identity` (works without auth) |
| Live events | websocket `/:/websockets/notifications`: library scans, playback, server shutdown |

## Library
| Purpose | Request |
| --- | --- |
| Sections | `GET /library/sections` (music = `type: artist`) |
| Artists / albums / tracks | `GET /library/sections/{id}/all?type=8|9|10` with paging headers `X-Plex-Container-Start` / `-Size` |
| Item + children | `GET /library/metadata/{ratingKey}`, `/children` |
| Recently added etc. | `GET /hubs/sections/{id}` |
| Search | `GET /hubs/search?query=…` |
| Sonically similar (needs sonic analysis) | `GET /library/metadata/{ratingKey}/nearest` (unverified) |
| Rescan one folder | `GET /library/sections/{id}/refresh?path=…` |

## Playback
| Purpose | Request |
| --- | --- |
| Direct file stream | `Media[].Part[].key` from track metadata, e.g. `/library/parts/{id}/{ts}/file.flac` (supports HTTP range requests) |
| Transcode | universal transcoder under `/music/:/transcode/universal/…` (see the log for working params) |
| Report progress | `GET /:/timeline?ratingKey=…&state=playing|paused|stopped&time=ms&duration=ms` |
| Mark played | `GET /:/scrobble?key={ratingKey}&identifier=com.plexapp.plugins.library` |
| Rate / like | `PUT /:/rate?key={ratingKey}&rating=10&identifier=com.plexapp.plugins.library` |
| Artwork | `/photo/:/transcode?url=…&width=…&height=…` |

## Playlists
| Purpose | Request |
| --- | --- |
| List | `GET /playlists?playlistType=audio` |
| One playlist | `GET /playlists/{id}` |
| Items | `GET /playlists/{id}/items` with container paging |
| Create | `POST /playlists?type=audio&title=…&smart=0&uri=server://{machineId}/com.plexapp.plugins.library/library/metadata/{keys}` |
| Add items | `PUT /playlists/{id}/items?uri=…` |
| Remove an item | `DELETE /playlists/{id}/items/{playlistItemID}` |
| Move an item | `PUT /playlists/{id}/items/{playlistItemID}/move?after={playlistItemID}` (no `after`: to the top) |
| Rename, description | `PUT /playlists/{id}?title=…&summary=…` |
| Cover | `POST /library/metadata/{id}/posters` with the image as the body |
| Delete | `DELETE /playlists/{id}` |
| History | `GET /status/sessions/history/all` |

## Players (Listen on)
Commands to a player use the Plex companion protocol: `GET
/player/<controller>/<command>?...&commandID=N` with the target's client id in
`X-Plex-Target-Client-Identifier`. They go straight to the player's address,
through the server (players registered in `/clients`), or through the Sonos
relay `https://sonos.plex.tv` with the account token.

| Purpose | Request |
| --- | --- |
| Players signed in to the account | `GET https://plex.tv/api/v2/resources?includeHttps=1` (entries whose `provides` has `player`) |
| Players registered with the server | `GET /clients` (`MediaContainer.Server[]`: `name`, `address`, `port`, `machineIdentifier`, `product`, `protocolCapabilities`) |
| Sonos linked to Plex | `GET https://sonos.plex.tv/resources` (XML `Player` elements) |
| What's playing where | `GET /status/sessions` (`Metadata[].Player.machineIdentifier`, `.state`) |
| Make a play queue | `POST /playQueues?type=audio&uri=server://{machineId}/com.plexapp.plugins.library/library/metadata/{keys}&key=/library/metadata/{start}&shuffle=0&repeat=0|1&continuous=0&own=1` |
| Read a play queue | `GET /playQueues/{id}` |
| Play it on a player | `/player/playback/playMedia?providerIdentifier=com.plexapp.plugins.library&machineIdentifier=&protocol=&address=&port=&token=&key=/library/metadata/{start}&offset={ms}&containerKey=/playQueues/{id}?window=100&own=1&type=music` |
| Controls | `/player/playback/{play,pause,stop,skipNext,skipPrevious}?type=music`, `seekTo?offset={ms}`, `setParameters?volume=0..100`, `shuffle=0|1`, `repeat=0|1|2` (off, one, all) |
| Follow it | `/player/timeline/poll?wait=0` (XML `Timeline` per type; the `type="music"` one has `state`, `time`, `duration`, `ratingKey`, `key`, `playQueueID`, `playQueueItemID`, `volume`, `shuffle`, `repeat`) |

## Findings log
Anything surprising goes here as it's tested (date, endpoint, what happened).

### 2026-10-13: first look at a real server (direct HTTP, read-only)

#### Discovery
- `GET plex.tv/api/v2/resources?includeHttps=1&includeRelay=1` with the account
  token: one owned server (PMS 1.43.4). Three connections, all
  `https://<ip-dashed>.<hash>.plex.direct:<port>` URIs: one `local: true`, one
  remote, one `relay: true` (port 8443). `httpsRequired: false`.
- `publicAddressMatches` was `false` even though the local connection
  answered. Don't use it to decide "at home"; race the connections instead.
- Race to `/identity` (no auth needed): local ~155 ms, remote ~117 ms, relay
  ~1.5 s. Remote can beat local on raw latency, so the race has to prefer
  local > remote > relay among connections that answer within a short grace
  window, not just take the first responder.
- The resource entry has its own `accessToken`. For an owned server the
  account token also works, but use the resource `accessToken` since that's
  correct for shared servers too.

#### Server root `/`
`myPlexSubscription: true` (Plex Pass), `musicAnalysis: 2` at server level,
`version`, `friendlyName`, `machineIdentifier`.

#### Sections
`GET /library/sections` → `Directory[]`. Music has `type: "artist"`,
`agent: tv.plex.agents.music`, `scanner: Plex Music`, a stable `uuid` (use it
to detect a replaced library), and `scannedAt`/`updatedAt`.
Section prefs `GET /library/sections/{id}/prefs`: `respectTags=true` (prefer
local metadata; the first-connect settings check reads this),
`musicAnalysis=false` (sonic analysis off on this library). Server prefs
`/:/prefs`: `MusicSeparateAlbumTypes=enabled` (release-type sections on artist
pages).

#### Counts
`X-Plex-Container-Start=0&X-Plex-Container-Size=0` returns only `totalSize`:
9 artists, 86 albums, 1,439 tracks. `X-Plex-Container-Size` as a query param
is ignored unless `X-Plex-Container-Start` is also given.

#### Albums (type 9) and release type
- The list endpoint doesn't include release type. Album detail
  (`/library/metadata/{id}`) has `Format: [{tag: "Album"|"EP"|"Single"}]` and
  `Subformat: [{tag: "Live"|"Compilation"|"Soundtrack"|"Demo"|"Remix"…}]`.
- Cheaper sync: list values with `GET /library/sections/{id}/format?type=9` and
  `/subformat?type=9` (here: Album, EP / Soundtrack), then
  `GET /library/sections/{id}/all?type=9&format={key}` (and `subformat=`) to
  tag albums in bulk instead of making N detail calls.
- Albums with no RELEASETYPE tag have no `Format` at all (e.g. the unreleased
  builds here). Treat them as "Album" for sectioning until the tags are fixed.
- Albums also have `Style`, `Mood` (detail only), `Genre` (the list has the
  first few), `studio`, `originallyAvailableAt`, `loudnessAnalysisVersion`,
  `UltraBlurColors` (colors computed by the server, usable for a header tint
  without decoding the art), `viewCount`, `skipCount`, `lastViewedAt`,
  `userRating`.
- Split albums exist: two albums with the same title and artist but different
  ratingKeys and track counts (their studio/year/album-artist tags differ).
  The tag check should flag these later; for now they show the way Plex shows
  them.

#### Tracks (type 10)
- `grandparentTitle` = album artist, `originalTitle` = track artist(s), comma
  joined, empty when it's the same as the album artist. `index`/`parentIndex`
  = track/disc.
- `Media[0]`: `audioCodec`, `container`, `bitrate`, `duration`;
  `Part[0].key` = `/library/parts/{id}/{ts}/file.{ext}` (direct stream),
  `Part[0].file` = server path, `size`.
- Codecs here: flac 1091, mp3 322, aac (mp4) 25, alac 1. One album can mix
  codecs (aac + mp3 + flac), so gapless has to work across codec changes.
- `bitDepth`, `samplingRate` and loudness are only in track detail
  (`Media[].Part[].Stream[]`, `streamType: 2`): `gain`, `albumGain`, `peak`,
  `albumPeak`, `loudness`, `lra`. This library has loudness analysis. Sync
  stores codec/bitrate from the list; bit depth, sample rate and gain are
  fetched lazily (when an album page opens or before play).
- Plex doesn't expose raw `ARCHIVE_*` or RELEASETYPE tags, so archive tags
  have to be read from the files themselves (the `tags/` module, later).
- `userRating` 10 = liked (one liked track here).

#### Playlists
`GET /playlists?playlistType=audio` → only smart playlists right now (the ones
Plexamp creates: "All Music", "Recently Added", liked tracks, …). Some are
duplicated with different counts, left over from an earlier library. Fields:
`smart`, `leafCount`, `composite` (mosaic art path).

#### History
`GET /status/sessions/history/all?librarySectionID={id}&sort=viewedAt:desc`
with container paging → 1,232 rows. Fields: `ratingKey`, `title`,
`grandparentTitle`, `viewedAt` (unix s), `accountID`, `deviceID`. No duration
or skip info, so the app needs its own local listen log.

#### Hubs
`GET /hubs/sections/{id}` → `music.recent.played.{id}`,
`music.recent.added.{id}`, `music.recent.artist`, `music.recent.genre`,
`music.popular`, `music.vault`, … each with `size` and `more`.

#### Sonic
`GET /library/metadata/{track}/nearest` → `size: 0` because sonic analysis is
off on this library. Features that use it need a fallback.

#### Search
`GET /hubs/search?query=…&sectionId={id}&limit=…` → `Hub[]` per type
(artist/album/track…). Only non-empty hubs have `Metadata`.

#### Write endpoints
`/:/timeline`, `/:/scrobble`, `/:/rate` and `/playQueues` weren't exercised.
Verify them against a real server when playback reporting and likes are built.

### 2026-10-13: Rust client live checks (`cargo test live_ -- --ignored`)
Read-only tests in `src-tauri/src/plex/live_tests.rs`. The write endpoints
(`/:/timeline`, `/:/scrobble`) are only unit-tested against wiremock.

- Race: with the grace-window rule the local connection won (≈35 ms once
  warm; the first cold probe was ≈200 ms vs remote ≈100 ms, so remote really
  can answer first). The relay didn't answer `/identity` within 4 s this time
  (≈1.5 s earlier). It's slow and unreliable, so it stays last.
- Incremental sync filter works: `GET /library/sections/{id}/all?type=10`
  with `updatedAt>=T` (sent as query key `updatedAt>` with value `T`, i.e.
  `updatedAt%3E=T` on the wire). T=0 returns every track, a future T returns
  0. Every track here had `updatedAt` within the last 90 days (the library
  was re-scanned recently), so a first incremental pass can be large.
- Format filters: `/library/sections/{id}/format?type=9` and
  `/subformat?type=9` return `Directory[]` with numeric `key`s; passing that
  key as `?format=` / `?subformat=` filters the album list. Here: Album 42,
  EP 3 (of 86 albums; the rest are untagged), Soundtrack 1.
- Section prefs (`/library/sections/{id}/prefs`) for the new music agent have
  no "use embedded tags" setting. The keys are `respectTags` (Prefer local
  metadata), `musicAnalysis`, `enableTrackOffsets`, `useExternalLyrics`,
  `artistBios`, `albumReviews`, `popularTracks`, `concerts`, `genres`,
  `albumPosters`, `albumSort`, `augmentWithSharedContent`, `hidden`. The
  first-connect settings check can only test `respectTags`.
- Search: `/hubs/search?sectionId=…` does not restrict results to that
  library (movie/show/episode hubs still come back), so hubs are filtered by
  type client-side (`PlexClient::search_music`). A 1-character query returns
  no music results; 4+ characters match.
- Direct stream: `GET /library/parts/{id}/{ts}/file.flac` with
  `Range: bytes=0-15` and the token as a header → `206`, `audio/flac`,
  `Accept-Ranges: bytes`. The token never needs to be in the URL.
- Universal transcoder (audio): `GET /music/:/transcode/universal/start.mp3`
  with `path=/library/metadata/{ratingKey}&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0&hasMDE=1&musicBitrate=128&maxAudioBitrate=128&session={uuid}`
  (+ `X-Plex-Session-Identifier` header) → `200 audio/mpeg`, bytes start
  streaming immediately. `GET /music/:/transcode/universal/stop?session={uuid}` → 200.
- Track detail loudness fields (`gain`, `albumGain`, `peak`, `loudness`) are
  all present, as are bit depth and sample rate (16/44100 on the sample).
- Photo transcode: `/photo/:/transcode?url={thumb}&width=200&height=200&minSize=1&upscale=1`
  with the token as a header → ≈8 KB JPEG.
- Websocket `/:/websockets/notifications` connects with `X-Plex-Token` as a
  handshake header (not in the URL). `playing` events from another client
  arrived within seconds.
- History paged with container headers → `totalSize` 1,232, same as the
  earlier count. Playlists: 8, all smart.

### 2026-10-13: concurrent file transfers get cut

Opening a second `/library/parts/...` transfer while another is still in
progress makes the server close the older socket after roughly 0.3 to 2 MB
(seen with Node fetch and reqwest alike). Distinct `X-Plex-Client-Identifier`
or `X-Plex-Session-Identifier` values don't change it; the newest transfer
always survives. A single transfer runs at full LAN speed (44 MB FLAC in under
3 s), so the client must download one file at a time: the playing track first,
then the next track, then pre-cache, each resuming by Range when preempted.

### 2026-10-13: photo transcode output format

`/photo/:/transcode?url={thumb}&width=320&height=320&minSize=1&upscale=1` on 60
album thumbs always came back as `image/jpeg`, about 19 KB each. `minSize=1`
fills the box, so non-square art comes back slightly larger on one side
(325x320, 320x427). `format=jpeg` changes nothing. `quality=82` makes the JPEGs
larger (about 23 KB), so the server default is already below that.
`format=webp&quality=82` returns `image/webp` at about 11 KB, and `format=png`
returns PNG at about 131 KB. Decoded size in the WebView is the same whatever
the format, so the app keeps the default JPEG.

### 2026-10-13: track ratingCount

Track items from `/library/sections/{id}/all?type=10` can carry an integer
`ratingCount`, the global listener count Plex gets from Last.fm. About 44% of
tracks had one; unreleased tracks never do. The app stores it per track and
ranks an artist's popular songs by it. Track artists other than the album
artist come in `originalTitle`, as one string such as `A, B` or `A feat. B`.

### 2026-10-14: artist list vs artist detail

Artist list items carry summary and art; Similar, Genre/Style/Mood and
UltraBlurColors only on /library/metadata/{id}. Similar entries there look
like `{id, filter, tag}`. The app reads Similar from the detail call when an
artist page opens and keeps it for a week, so sync stays one paged list call.

### 2026-10-14: artist MusicBrainz ids

On 5 sampled artists, list items have only `guid` (`plex://artist/…`) and no
`Guid` array. Artist detail (`/library/metadata/{id}`) has `Guid` with one
`mbid://<36-char uuid>` entry, with or without `includeGuids=1`. The artist
info lookups read the MBID from that detail call and cache it for 30 days.

### 2026-10-14: playlist writes

Read-only checks on the real server:
- `GET /playlists/{id}` returns one `Metadata` with `summary` (the
  description, an empty string when unset), `updatedAt`, `leafCount`,
  `duration` and `composite`. No `thumb` unless someone chose a cover, so the
  app shows `thumb` when there is one and the `composite` mosaic otherwise.
- `GET /playlists/{id}/items` on a smart playlist has no `playlistItemID`
  on its items. The server has no ordinary playlists yet, so the field on
  ordinary playlist items is taken from the public API docs: a number per
  row, used to remove or move that row.
- Orphaned smart playlists left from an old library answer 500 on
  `/items`, as noted before.

Not tried against a real server yet (unit tested against wiremock and a
fake only). Check once on a throwaway playlist, then delete it:
- Create with `uri=…/library/metadata/{k1},{k2}`: the reply's `Metadata[0]`
  holds the new `ratingKey`, and the songs keep the order given.
- Plex needs at least one item to create a playlist (the app waits for the
  first song). Worth confirming whether an empty `uri` is refused.
- Add appends at the end, in the order given.
- Remove and move by `playlistItemID`; move without `after` goes to the top.
- `PUT /playlists/{id}?title=&summary=` changes only the fields sent.
- Poster upload as a raw JPEG body with `Content-Type: image/jpeg`.
- Delete.
- Whether `updatedAt` changes on every edit (the sync refetches a playlist's
  items when count, duration or `updatedAt` change; a reorder alone only
  changes `updatedAt`).

### 2026-10-14: ratings (likes)

Read-only checks on the real server:
- Track list items (`/library/sections/{id}/all?type=10`) carry `userRating`
  and `lastRatedAt` (unix seconds) on rated tracks, and `Genre` tags (up to
  two; about 99% of tracks had one). `Mood` wasn't on any list item, so the
  Liked Songs chips are mostly genres.
- The filter `userRating>=1` (sent as query key `userRating>` with value `1`)
  returns only rated tracks; `userRating>=10` returns only five-star ones.
  It's a small list, so every sync reads it to pick up likes made in other
  Plex apps.
- On the one rated track, `updatedAt` was later than `lastRatedAt`, so a
  rating doesn't reliably show up in the incremental `updatedAt` filter.
  That's why ratings get their own pass.

Not tried against a real server yet (unit tested against wiremock and a
fake only). Check once on a song you don't mind rating, then put it back:
- `PUT /:/rate?key={ratingKey}&identifier=com.plexapp.plugins.library&rating=10`
  sets five stars and `lastRatedAt`.
- `rating=-1` clears the rating (the app's unlike).
- Whether a rating change sends a websocket `timeline` event (the app syncs
  on those, so likes from other apps would show without waiting).

### 2026-10-14: players (Listen on)

Read-only checks with the probe script:
- plex.tv resources list each signed-in player with `provides` containing
  `player`. Plexamp reports `client,player,pubsub-player`; other Plex apps
  report `client,player`. Idle players show `presence: false`, and each has
  its own `accessToken`.
- Plexamp lists its companion port 32500 over plain http. Some other Plex
  apps list https on 32400 instead.
- `GET /clients` answered `{"MediaContainer":{"size":0}}` with nothing open,
  and `GET /status/sessions` had no sessions, so the shapes for those come
  from public protocol references, not from this server yet.

Not tried against real players (builders only test against wiremock fakes;
anything that plays on a real device needs an OK first). Check once:
- `GET /clients` with Plexamp open on the same network: the player shows up
  with `address`, `port` and `protocolCapabilities`.
- `GET https://sonos.plex.tv/resources` on an account with and without Sonos
  linked (shape, and what a no-Sonos account gets).
- `POST /playQueues?type=audio&uri=...&key=...` with a list of rating keys:
  `playQueueID` comes back and the order is kept. Also the longest key list
  the server takes (Platter caps at 300).
- `playMedia` to Plexamp directly at its 32500 address with the player's own
  token, then through the server with `X-Plex-Target-Client-Identifier`:
  which one Plexamp accepts, and whether `offset` starts it at that spot.
- `/player/timeline/poll?wait=0` on Plexamp: XML or JSON, and whether `time`
  moves while playing. Then `pause`, `seekTo`, `setParameters?volume=` and
  `stop`.
- Platter as a player (setting on): Plexamp and Plex Web list it, can start
  music on it with `playMedia`, and follow it with `timeline/poll`. Whether
  they need `timeline/subscribe` pushes (Platter accepts subscribe but only
  answers polls).
