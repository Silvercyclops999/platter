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
| Home users | `GET https://plex.tv/api/v2/home/users` | `{ users: [...] }` with `uuid`, `title`, `friendlyName`, `thumb`, `admin`, `restricted`, `protected` (needs a PIN to switch) |
| Switch Home user | `POST https://plex.tv/api/v2/home/users/{uuid}/switch` | Form body `pin=NNNN` when the user has one. Answers the user with `authToken`; 401/403 means a wrong or missing PIN. Not called against the real account |
| Plex Pass | `GET https://plex.tv/api/v2/user` | `subscription.active`, or a `plexpass` entry in `roles`. Label restrictions on shares need it |
| A server's libraries, plex.tv ids | `GET https://plex.tv/api/servers/{machineId}` | XML. `Server/Section` with `id` (plex.tv's), `key` (the server's section id), `type`, `title`. Shares name sections by `id` |
| Share a server | `POST https://plex.tv/api/servers/{machineId}/shared_servers` | JSON body, see "Sharing a server" below. Not sent to a real account |
| Who a server is shared with | `GET https://plex.tv/api/servers/{machineId}/shared_servers` | XML `SharedServer` per person with `Section ... shared="1"` children. Carries `accessToken`, dropped while reading |
| Remove a share | `DELETE https://plex.tv/api/servers/{machineId}/shared_servers/{id}` | They stay a Plex friend |
| Last seen | `GET https://plex.tv/api/users` | XML `User/Server` with `machineIdentifier` and `lastSeenAt` (unix s) |
| Invites received | `GET https://plex.tv/api/invites/requests` | XML `Invite` with `id`, `friend`, `home`, `server` (0/1), `friendlyName`, `username`; `Server name` children |
| Accept an invite | `PUT https://plex.tv/api/invites/requests/{id}?friend=&home=&server=` | The flags as listed on the invite. Not sent to a real account |

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
| Read a play queue | `GET /playQueues/{id}?window=600&own=1` (`playQueueVersion`, `playQueueSelectedItemID`, `playQueueTotalCount`, `Metadata[]` with `playQueueItemID` and `ratingKey`) |
| Add to it | `PUT /playQueues/{id}?uri=server://{machineId}/com.plexapp.plugins.library/library/metadata/{keys}&next=0` (answers the queue after the change) |
| Move an entry | `PUT /playQueues/{id}/items/{playQueueItemID}/move?after={playQueueItemID}` |
| Remove an entry | `DELETE /playQueues/{id}/items/{playQueueItemID}` |
| Tell the player it changed | `/player/playback/refreshPlayQueue?playQueueID={id}&type=music` |
| Play it on a player | `/player/playback/playMedia?providerIdentifier=com.plexapp.plugins.library&machineIdentifier=&protocol=&address=&port=&token=&key=/library/metadata/{start}&offset={ms}&containerKey=/playQueues/{id}?window=100&own=1&type=music` |
| Controls | `/player/playback/{play,pause,stop,skipNext,skipPrevious}?type=music`, `seekTo?offset={ms}`, `setParameters?volume=0..100`, `shuffle=0|1`, `repeat=0|1|2` (off, one, all) |
| Follow it | `/player/timeline/poll?wait=0` (XML `Timeline` per type; the `type="music"` one has `state`, `time`, `duration`, `ratingKey`, `key`, `playQueueID`, `playQueueItemID`, `playQueueVersion`, `volume`, `shuffle`, `repeat`) |
| Token for another device | `GET /security/token?type=delegation&scope=all` (`MediaContainer.token`, a transient token) |

Casting to a Chromecast uses Plex's Cast receiver (app id `9AC194DC`). After
launching it, the sender sends a media `LOAD` whose `contentId` is
`/library/metadata/{start}` and whose `customData` carries `offset` (seconds),
`containerKey` (`/playQueues/{id}?own=1&window=200`) and a `server` object
(`machineIdentifier`, `protocol`, `address`, `port`, `accessToken`, `user`).
Play, pause, next, previous, stop and seek go on the `urn:x-cast:plex`
namespace as `{type: "PLAY"}` and so on, seek with `currentTime` in seconds.
Status comes back as standard media status messages.

## Findings log
Anything surprising goes here as it's tested (date, endpoint, what happened).

### 2026-10-21: plex.direct names on a network whose DNS drops them
- A Mac on a network with rebind protection got no answer for every saved
  `*.plex.direct` connection, so the server looked offline. A public resolver
  answered with the address the name spells. Platter still asks DNS first,
  but when it gets nothing, an error, 0.0.0.0 or no answer within 1.5 s, it
  reads the address from the name (`10-0-0-5.<hash>.plex.direct` is 10.0.0.5,
  IPv6 uses `-` for `:`). TLS still checks the certificate against the full
  name.

### 2026-10-19: a library whose audio playlists are all smart (read-only)
- `GET /playlists?playlistType=audio` on a real server answered 8 playlists,
  every one with `smart: true` (leafCount 1 to 1439). None of them takes songs
  added by hand, so an Add to playlist list is rightly empty there; the phone's
  sheet says why and keeps New playlist.

### 2026-10-14: several items in one metadata call (read-only)
- `GET /library/metadata/{k1},{k2},{k3}` with two album keys and a track key
  answered one `Metadata` list in the asked order with mixed `type`s
  (`album`, `track`), each with `librarySectionID`. Album entries carry
  `Format` and `Style` like the single-item detail. Live library updates use
  it to read the items a batch of `timeline` entries names, 50 at a time.

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
  Platter doesn't keep `file` in its cache. The Lidarr setup check lists every
  track live and compares these paths and sizes with what Lidarr sees.
  Music sections in `/library/sections` carry `Location[].path`, the
  section's folders in the server's own path form.
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
`ratingCount`. About 44% of tracks had one; unreleased tracks never do. Track
artists other than the album artist come in `originalTitle`, as one string
such as `A, B` or `A feat. B`.

Correction (2026-10-14): `ratingCount` is not a global listener count and
can't rank songs. See the entry below.

### 2026-10-14: ratingCount and track MusicBrainz ids

On one large artist (770 tracks), 63 tracks had `ratingCount`, and they are
the same 63 that have a `Guid` `mbid://` entry (with `includeGuids=1` on the
track list). The counts are not on one scale: a well-known single had about
600 while every track of one recent album had between 130,000 and 320,000, so
ranking by them put that one album at the top. Plex's own most popular
tracks hub for the artist sorts the same way. The app no longer ranks by
`ratingCount` (it only breaks ties between songs you've played equally);
Popular uses the ListenBrainz top recordings for the artist instead,
matched to library tracks by title.

The track `mbid://` is a MusicBrainz track id, not a recording id: on 3
sampled tracks the MusicBrainz recording lookup answered 404 and a search by
track id found each one. None of the 63 matched a ListenBrainz recording id,
so the app doesn't store them.

### 2026-10-14: list items with MusicBrainz ids, and other servers

Album and track list calls send `includeGuids=1` so each item comes with its
`Guid` array; the `mbid://` entry is kept to find the same album or song on
another server. Nothing else changes in the answer.

Other servers on the account are reached the same way as the connected one:
their connections from the resources list, local address first, then remote
access, then the relay. A shared server takes its own `accessToken` from the
resources list. Each server's item ids are its own; the app puts the
server's tag on every id it reads and takes it off before a request, and
refuses to send another server's id to a server. Not yet checked against a
real second server.

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

### 2026-10-14: album MusicBrainz ids

On 10 sampled albums, album detail (`/library/metadata/{id}`) has `Guid`
with one `mbid://<36-char uuid>` entry. Checked 3 against MusicBrainz: each is
a release id (the release lookup answers, the release-group lookup doesn't),
and Cover Art Archive's `/release/<id>/front-500` redirects to an image for
all 3. Discord status uses that URL for covers.

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
- Editing a play queue a player is using: where `PUT /playQueues/{id}?uri=`
  puts the new entries (Platter moves them after the right entry when they
  land elsewhere), whether move and delete answer the queue, whether
  `playQueueVersion` goes up on each change and shows in the player's
  timeline, and that `refreshPlayQueue` makes Plexamp pick up the change
  without restarting the song.
- `/player/timeline/poll?wait=0` on Plexamp: XML or JSON, and whether `time`
  moves while playing. Then `pause`, `seekTo`, `setParameters?volume=` and
  `stop`.
- Platter as a player (setting on): Plexamp and Plex Web list it, can start
  music on it with `playMedia`, and follow it with `timeline/poll`. Whether
  they need `timeline/subscribe` pushes (Platter accepts subscribe but only
  answers polls).

### 2026-10-14: Platter devices in plex.tv

Read-only checks with the probe script (booleans and counts only):
- Running Platter installs are not in `/api/v2/resources` at all (they
  publish no connections), but `/devices.xml` lists them with
  `provides="player"`, `product="Platter"` and `lastSeenAt`.
- `authToken` from `/api/v2/user` and the owned server's `accessToken` in
  resources are just the token the request was made with. That made the
  token look account-wide, which turned out to be wrong (next section).

Not tried yet (needs Platter running on two computers):
- `PUT /devices/<client id>?Connection[][uri]=http://<lan address>:32520`
  (what Platter now sends every 5 minutes while showing itself): the
  computer then shows in `/api/v2/resources` with those connections and a
  fresh `lastSeenAt`.

### 2026-10-14: every device has its own token

Two Platter computers on one account couldn't control each other: each
signed its requests with a key from its own token and checked others with
the same key. Read-only checks with the probe script (token values never
printed, only compared):
- `GET /api/v2/devices` (JSON, account token) lists every device signed in
  to the account: `id`, `name`, `product`, `platform`, `provides`,
  `clientIdentifier`, `createdAt`, `lastSeenAt`, `presence`, `connections`
  and `token`. Every entry has a 20 character token and no two are the
  same. The token the probe uses matches exactly one entry, so the listed
  token is the one that device really uses. Without a token the endpoint
  answers 401.
- A device can be listed twice under one `clientIdentifier` with different
  tokens (seen once, a TV app), so every listed token counts.
- `/api/v2/resources` lists clients (Plexamp, Platter) with `accessToken`
  empty; only the owned server has one, and it's the caller's own token.
- `/api/v2/home/users` lists the Home's users, each with its own id, so a
  Home user's account tag differs from the admin's. Which list a device
  shows up in after switching Home users wasn't tried (it would mean
  switching on a real device).

What Platter does with it: a receiver fetches this list with its own token
(cached 5 minutes, fetched again for a sender it doesn't know or whose key
doesn't match, at most every 10 seconds), keeps only a hash of each token,
and checks the sender's signature against the keys listed for its client
id. No token is sent between devices. A device signed in as another account
or another Home user isn't in the list under that token and is refused.

### 2026-10-14: loudness fields
- Track detail `Media[0].Part[0].Stream[]` with `streamType: 2` carries
  `gain`, `albumGain`, `peak`, `albumPeak`, `loudness` and `lra`, all as
  strings (for example `"-10.93"`, `"1.015710"`). The lenient number parser
  reads them.
- `gain` and `albumGain` are dB to apply; `peak` and `albumPeak` are linear
  and can be a little over 1 for lossy files. On the sample track gain plus
  loudness came to about -21, so Plex's reference is its own, and the player
  applies the gain as given.
- None of these are in the list endpoints, so the player fetches track
  detail once per track on first play and caches the four values.

### 2026-10-14: casting (not yet checked live)
- The Cast path above follows how existing open-source Plex cast senders
  drive Plex's receiver. Nothing has been sent to a real device yet. To
  check with a real Chromecast: that the receiver starts and plays the play
  queue at the offset, which field of its media status carries the playing
  song's rating key (Platter reads `customData.ratingKey`, `customData.key`
  or a `/library/metadata/{id}` `contentId`), and whether it honours
  `QUEUE_UPDATE` for shuffle and repeat.
- `/security/token?type=delegation&scope=all` hasn't been called against
  the real server (it creates a token). Platter falls back to the server
  token when it gets no answer; check that the transient token comes back
  and that the receiver accepts it.

### 2026-10-14: account profile picture
- `GET plex.tv/api/v2/user` returns `thumb` as
  `https://plex.tv/users/<id>/avatar?c=<n>`. The `c` value changes when the
  picture does, so it works as a cache key.
- That URL needs no token. Without one it answers 302 to a PNG on
  `assets.plex.tv` (about 400 KB, full size). Platter fetches it through the
  art cache, which scales it down.
- On a Plex Home account the same call has `home: true`, `homeSize` and
  `homeAdmin`, and `thumb` is the picture of the user the token belongs to.

### 2026-10-14: Home users (read-only)
- `GET plex.tv/api/v2/home/users` answers `{ id, name, guestUserID,
  guestUserUUID, guestEnabled, subscription, users }`. A Home of six here.
- Each user has `id`, `uuid`, `title`, `username` (null for a managed user),
  `email`, `friendlyName` (often null), `thumb`, `hasPassword`, `restricted`,
  `admin`, `guest`, `protected` and `updatedAt`. `protected` is the PIN flag;
  `hasPassword` only says the user has a plex.tv password.
- The admin's entry also carries a `pin` field. Platter never reads it.
- Switching (`POST .../home/users/{uuid}/switch`) wasn't tried against the
  real account; it's covered by faked plex.tv tests only. Check it once by
  hand with a test Home user.

### 2026-10-14: lyrics streams
- Track detail lists lyrics as `streamType: 4` streams next to the audio
  stream, each with `key: "/library/streams/{id}"`, `codec` and `format`
  (`"lrc"` or `"txt"`), `provider` (the agent that supplied them, for
  example `com.plexapp.agents.lyricfind`), `timed: "1"` on timed ones,
  `minLines`, `displayTitle` and `extendedDisplayTitle` ("LRC (External)").
  A track can have both an LRC and a TXT stream. Of 80 well-played tracks,
  31 had lyrics, every one with an LRC stream and 9 with a TXT stream too.
- `GET /library/streams/{id}` with `Accept: application/json` returns
  `MediaContainer.Lyrics[]` with `provider`, `timed` (bool), `minLines`,
  `author` and `by`, and `Line[]`. Timed lines carry `startOffset` and
  `endOffset` in ms and one `Span` with the whole line's `text` and the same
  offsets. No word timing was seen from this provider. Plain (TXT) lines
  have only `Span[].text`.
- Without the JSON accept header the same path returns the raw file (served
  as `text/html`): LRC with `[mm:ss.xx]` tags and header tags such as
  `[by:...]` and a copyright line, or plain text.
- After a burst of a few dozen lyrics requests the server started answering
  404 for every lyrics stream (ones that had returned 200 minutes before).
  The app treats a failed lyrics stream as "couldn't ask" (Retry), not "no lyrics", and never caches it.
- Telling a sidecar file from agent lyrics: of 65 well-played tracks checked
  again, the one with lyrics had `provider: "com.plexapp.agents.lyricfind"`.
  No sidecar `.lrc` or `.txt` was in that sample, so the app's rule is an
  assumption to confirm: a stream naming `com.plexapp.agents.localmedia`, or
  no provider at all, is a file next to the song; any other agent is Plex's
  own lyrics.
### 2026-10-14: file header reads for tags

`cargo test live_tags -- --ignored --nocapture` read 9 parts (3 FLAC, 3 MP3,
3 MP4) one at a time with nothing playing. Every part answered
`Range: bytes=0-65535` with `206` and a `Content-Range` total. FLAC and MP3
needed only that read; all three MP4s needed one more (the `moov` atom, or its
`udta`, sits past the first 64 KB). None of the sampled files carried
`ARCHIVE_*` tags or an explicit flag yet, so the scan finds nothing to show
until the library is tagged. Not checked yet: that the part key's middle number
changes when a file is retagged (the tag cache relies on it to read again).

### 2026-10-14: album release date and label

The album list (`/library/sections/{id}/all?type=9`) carries
`originallyAvailableAt` as a plain `YYYY-MM-DD` date and `studio` (the label).
Here every one of 86 albums had a date and 44 had a studio. The app stores both
for the album page footer and takes the first ten characters of the date, in
case a time follows it.

### 2026-10-14: albums Plex couldn't match

Every album has a `guid`. When Plex matched it to its music agent the guid is
`plex://album/<id>`, and with `includeGuids=1` the `Guid` array holds an
`mbid://` entry. When it found no match the guid is `local://<n>` and `Guid` is
empty. Here 41 of 86 albums were `local://` and 45 were `plex://`, and the 41
are exactly the albums with no `studio`; none of them had a release type
either. In this library those are the unreleased albums. The app stores
whether each album was matched (`albums.matched`) and, unless the user turns it
off in Settings, counts an unmatched album with no release type and no
ARCHIVE_VERSION as probably unreleased. For cache rows saved before the guid
was kept, no release type and no studio stands in until the next full sync.

### 2026-10-14: genre, style and mood tags (mixes)

Album and track list items carry `Genre` only: 85 of 86 albums and 1433 of
1439 tracks had one (at most two per track, 9 distinct). `Style` and `Mood`
are on none of the list items, but album detail has them (one album: 1 genre,
4 styles, 10 moods). The tag directories (`/library/sections/{id}/style?type=9`
and `/mood?type=9`) list 24 styles and 151 moods for albums; tracks list 161
moods and no styles. The app reads album styles and moods the way it reads
format: one filtered album list per tag value (`?style=<key>`,
`?mood=<key>`), four at a time, at the start of a full sync. Track moods are
left to album moods (161 more lists for little gain).

The sonically similar endpoint (`/library/metadata/{id}/nearest`) answered
with no items here: sonic analysis is off on this server, so mixes and radio
don't use it.

### 2026-10-14: downloads

Downloads use the same two requests as playback, one file at a time through
the transfer gate. Original quality is a plain `GET` of the part with the token
as a header, resumed with `Range: bytes=N-` after a break (the part answers
`206` with a `Content-Range` total, see the direct stream note above). MP3
qualities use `start.mp3` with `musicBitrate` and `maxAudioBitrate` set to
320, 192 or 128 and a fresh session id, then `stop` with that session. A
transcode has no length and no Range, so a broken one starts over.

With nothing playing, `GET /music/:/transcode/universal/decision` with the same
query as `start.mp3` (192 kbps, one FLAC track at 892 kbps) answered `400` with
an HTML body, with and without `X-Plex-Session-Identifier` and a client
profile. So the app doesn't ask for a decision first; it relies on the
`start.mp3` check from the first log entry. Not checked yet: a full MP3
download's size against the bitrate estimate the storage limit uses.

### 2026-10-14: sessions, history and accounts (friend activity)

Read with the owner's token. `/status/sessions` had no session at the time
(`size` 0), so its music entries are read the way other Plex clients document
them: `type` "track", `ratingKey`, `title`, `grandparentTitle`, `parentTitle`,
`parentRatingKey`, `parentThumb`, `User` (`id`, `title`, `thumb`) and `Player`
(`state`). Not yet seen live: whether `User.id` is the server account id
(`1` for the owner) as in history; the app reads it that way.

`/status/sessions/history/all?librarySectionID=<music>&sort=viewedAt:desc`
with a page of 300 returned 300 of 1273 rows, all `type` "track", with
`historyKey`, `key`, `ratingKey`, `librarySectionID`, `parentKey`,
`grandparentKey`, `title`, `parentTitle`, `grandparentTitle`, `thumb`,
`parentThumb`, `grandparentThumb`, `grandparentArt`, `index`, `parentIndex`,
`viewedAt`, `accountID` and `deviceID`. There's no `parentRatingKey`; the album
id is the last part of `parentKey`. Those rows came from 2 accounts, and 73 of
them were from the last 7 days.

`/accounts` listed 46 accounts with `id`, `key`, `name`, `thumb` and some
language fields; every `thumb` was empty, so avatars come from the live
session's `User.thumb` when there is one and fall back to the name's letter.
plex.tv's `/api/v2/home/users` has `thumb` for Home users (6 here) but isn't
used yet, and `/api/v2/friends` answered 410.

### 2026-10-14: smart playlists

Read only, from the 8 audio smart playlists on the server. `/playlists?playlistType=audio`
lists them with `smart` true but no query. `/playlists/{id}` adds `content`,
the query as a URI: `library://x/directory/` followed by the percent-encoded
path, for example
`/library/sections/<id>/all?type=10&track.userRating%3E%3E=4&group=guid&having=min(album.originallyAvailableAt)&sort=track.lastRatedAt`
(after one decoding). The operator sits in the key, encoded again: `%3E%3E=`
is `>>=`. Shapes seen: an empty filter (every song in the section),
`track.userRating>>=4`, `track.lastViewedAt<<=-1mon`,
`track.lastViewedAt>>=-2w`, `album.addedAt>>=-1mon`, and sorts
`album.addedAt:desc`, `track.lastViewedAt:desc`, `lastViewedAt`,
`track.lastRatedAt`. Four of them carry `group=guid&having=min(album.originallyAvailableAt)`,
Plex Web's "one copy of each song".

`/library/sections/<id>/all?type=10&includeMeta=1` lists the filter fields
per type with their kinds and the operators per kind: tag `=` `!=`; integer
`=` `!=` `>>=` `<<=`; string `=` (contains) `!=` `==` (is) `!==` `<=` (begins
with) `>=` (ends with); boolean `=` `!=`; date `<<=` (before) `>>=` (after),
with relative values like `-30d`, `-2w`, `-6mon`, `-1y`. Track fields:
`track.mood`, `track.genre` (tags), `track.title`, `track.viewCount`,
`track.lastViewedAt`, `track.skipCount`, `track.lastSkippedAt`,
`track.userRating` (0 to 10), `track.lastRatedAt`, `track.addedAt`,
`track.mediaSize`, `track.mediaBitrate`, `track.audioCodec`, `track.trash`,
`track.location`, `track.source`. Album fields include `album.title`,
`album.year`, `album.decade`, `album.genre`, `album.style`, `album.mood`,
`album.format`, `album.subformat`, `album.addedAt`,
`album.originallyAvailableAt`; artist fields include `artist.title`,
`artist.genre`, `artist.style`, `artist.mood`. There is no track length
field. Tag filters take tag ids: `/library/sections/<id>/genre?type=10`
(and `type=9` for album tags) lists them as `Directory` entries with a
numeric `key` and a `title`.

Not checked against the server, because they write: creating with
`POST /playlists?type=audio&title=<title>&smart=1&uri=server://<machine>/com.plexapp.plugins.library/library/sections/<id>/all?<query>`
and changing the rules with `PUT /playlists/{id}/items?uri=<the same>`, as
other Plex clients do, and "match any" as `push=1&<a>&or=1&<b>&pop=1`. The
app tests these with a fake server only. `limit=<n>` caps the songs.
Groups of rules nest the same way, each its own `push=1 ... pop=1` joined by
`and=1` or `or=1`, and a number range is a group of `>>=` one below the low
end and `<<=` one above the high end (Plex leaves both ends out); a range
from 0 is just the `<<=`. Not yet checked against a server either.
`track.title` takes the string operators. Last skipped is read from the
listen history on this computer, like Skips, not from `track.lastSkippedAt`,
so the count the editor shows is the one the playlist fills.

### 2026-10-14: can Plex carry listen along between accounts?

Listen along needs a channel between people on different Plex accounts
where nobody learns anyone else's address, invites carry only a random
token, and nobody gets more of the host's server than their own access
already gives them. Three Plex paths were looked at, read-only:

- Relay connections. plex.tv resources list a server's connections with
  `protocol`, `address`, `port`, `uri`, `local`, `relay` and `IPv6`; the one
  with `relay: true` answered `/identity` here. A relay only proxies HTTP to
  a Plex Media Server that registered it. Players and apps have no relay
  entry (Plexamp and Platter list only `local` or direct connections), so
  there's no relay address one app could reach another app through.
- plex.tv pubsub. Plexamp lists `provides: "client,player,pubsub-player"`;
  other players list `client,player`. The pubsub service isn't documented,
  and what's known publicly is that it carries control messages between
  devices on one account. Nothing suggests one account can send to another
  account's device through it, and using it would mean sending Plex tokens
  to an undocumented service on behalf of the user.
- The shared server itself. `GET /playQueues/<id>` for an id that isn't
  yours answers an empty container (`size: 0`, `playQueueVersion: -1`), and
  `GET /playQueues` lists nothing; play queues belong to the account that
  made them. Playlists are per account too, and `/status/sessions` (position
  and state) only answers the owner. A guest would also have to write to the
  host's server to send anything back. The server's root says
  `companionProxy: true`, but that forwards player commands from the server
  to a player's own address, which means the server connecting to each
  guest's computer.

None of these works across accounts without either exposing an address or
writing to the server, so listen along uses an ntfy relay (behind a relay
interface, so a Plex path can be added if Plex ever offers one). What it
does use from Plex: each guest's own access to the server, rating keys, and
the server's machine id, hashed with the session id before it's sent.

### 2026-10-16: sharing a server (not sent to a real account)

Share my music and Let them listen from my server use plex.tv's older
`/api` sharing endpoints. Nothing below was sent to a real Plex account:
the shapes come from python-plexapi (`MyPlexAccount.inviteFriend`,
`_getSectionIds`, `_filterDictToStr`, `pendingInvites`, `acceptInvite`,
`MyPlexServerShare`) and are tested against a faked plex.tv
(`plex::sharing::tests`, `social::sharing::tests`). The ids, names and
tokens here are made up.

Sharing a server:

```
POST https://plex.tv/api/servers/{machineId}/shared_servers
X-Plex-Token: <account token>
Content-Type: application/json

{
  "server_id": "{machineId}",
  "shared_server": { "library_section_ids": [901, 903], "invited_email": "robin_listens" },
  "sharing_settings": {
    "allowSync": "0", "allowCameraUpload": "0", "allowChannels": "0",
    "filterMovies": "", "filterTelevision": "",
    "filterMusic": "label!=platter-unreleased"
  }
}
```

- `library_section_ids` are plex.tv's section ids from
  `GET /api/servers/{machineId}` (`Section id`), not the server's own
  section keys. Platter looks them up each time and refuses any section
  whose `type` isn't `artist`, before plex.tv is asked.
- `invited_email` takes a Plex username or an email.
- `filterMusic` is Plex's restriction string: keys `label`, `label!`,
  `contentRating`, `contentRating!`, values joined with `%2C`, keys joined
  with `|`. `label!=x` leaves out items labelled `x`. Restrictions need Plex
  Pass on the owner's account.
- Answers seen in other clients: 200/201 with an XML `SharedServer`; 400 with
  a message when the user already has access (Platter looks for "already");
  404 or 422 for an unknown user. Still to check against a real account.

Unreleased albums get their label on the server with the tag edit Platter
already uses (`label[N].tag.tag`, every label the album keeps plus the new
one, and `label.locked=1`):

```
GET /library/metadata/{albumKey}                      (Label[] it has now)
PUT /library/sections/{s}/all?type=9&id={albumKey}&label[0].tag.tag=Vinyl&label[1].tag.tag=platter-unreleased&label.locked=1
```

Invites received: `GET /api/invites/requests` answers

```
<MediaContainer size="1">
  <Invite id="8001" createdAt="1760000000" friend="1" home="0" server="1" username="alex_hosts" friendlyName="Alex">
    <Server name="Alex's Music" numLibraries="1"/>
  </Invite>
</MediaContainer>
```

and accepting is `PUT /api/invites/requests/8001?friend=1&home=0&server=1`.
After that the server shows in `GET /api/v2/resources` with `owned: false`
and its own `accessToken`, and Platter races its connections like any other
server (`PlexState::refresh_servers`, `client_for_tag`).

To check with a real account (owner and a second test account, with
permission): the share's answer and its error bodies, whether
`label!=` really hides labelled albums from the friend (and their search),
whether Plex keeps `label.locked`, and how soon an invite shows in
`/api/invites/requests`.

### 2026-10-15: editing metadata (reads checked, writes not sent)

Only GETs were sent to the real server. The writes below follow the shapes
other Plex clients use and are tested against a faked server.

- `GET /library/metadata/{id}/posters` answers `MediaContainer` with `size`
  and `Metadata[]`, each `{ key, ratingKey, thumb, selected, provider }`.
  The one in use has `selected: true`, a `metadata://posters/...` (or
  `upload://...`) `ratingKey`, and server paths for `key` and `thumb`. The
  others are suggestions: `ratingKey` and `key` are the image's https link
  and `thumb` is an `images.plex.tv` preview. Providers seen on albums:
  `embedded`, `caa`, `lastfm`, `discogs`, `fanarttv`, `tivo`; one album had
  a single suggestion and none selected. Artists listed up to 13, one
  selected.
- `GET /library/metadata/{id}/arts` on an artist has the same shape (3
  backgrounds, `fanarttv`).
- `GET /library/metadata/{id}/matches?manual=1` answers `SearchResult[]`
  (not `Metadata`). Albums: `{ type, guid, name, parentName, parentGUID,
  score, year, lifespanEnded, SearchResult[] }`, the inner list being the
  candidate's tracks (`id, type, index, guid, name, matched`). Guids are
  `mbid://...`, scores 0 to 100, no thumbnails. Adding `title` and `year`
  changed the list (4 results instead of 3). Artists: `{ type, guid, name,
  score, area?, beginArea?, lifespanBegin?, lifespanEnded }`.
- Album detail has `studio`, `originallyAvailableAt`, `summary`,
  `librarySectionID`, `Genre`, `Style`, `Mood`, and no `titleSort` when
  none is set. No album in the music section had a `Field` list, so no
  field was locked; when one is, the detail carries
  `Field: [{ name, locked }]`. Guids were `plex://` (matched) or `local://`.

Writes used: `PUT .../poster?url=<ratingKey>` (or `/art`) selects one,
`POST .../posters` with the image as the body uploads one and selects it,
`POST .../posters?url=<link>` has the server fetch it,
`PUT .../match?guid=&name=&year=`, `PUT .../unmatch`,
`PUT /library/sections/{s}/all?type=8|9&id=&<field>.value=&<field>.locked=1`
(tags as `genre[0].tag.tag=` and removals as `genre[].tag.tag-=a,b`),
`PUT /library/metadata/{target}/merge?ids=a,b` (the target keeps its name
and photo), `PUT .../split` and `PUT .../refresh`. A 401 or 403 on any of
them is read as "this account can't edit the server".

