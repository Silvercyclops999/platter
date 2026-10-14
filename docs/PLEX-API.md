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
| Create | `POST /playlists?type=audio&title=…&smart=0&uri=server://{machineId}/com.plexapp.plugins.library/library/metadata/{keys}` |
| Add items | `PUT /playlists/{id}/items?uri=…` |
| History | `GET /status/sessions/history/all` |

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
