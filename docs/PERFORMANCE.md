# Performance

Platter aims to stay light: an installer under 15 MB, under 150 MB of private
working set with the Library open, about 0% CPU while idle and paused, and a
cold start under 1.5 s. This page records what was measured and what changed
in the last pass over it. Runtime memory and CPU are measured on the
installed app.

## Sizes

Windows release build (`npm run tauri build`, without the transcription
feature), before and after this pass:

| | Before | After |
|---|---|---|
| NSIS installer | 8.67 MB | 8.91 MB |
| MSI installer | 12.21 MB | 12.57 MB |
| platter.exe | 26.6 MB | 27.4 MB |
| JS parsed at start | 886 KB (262 KB gzip) | 730 KB (220 KB gzip) |
| JS loaded when first used | none | 200 KB in 12 chunks |
| CSS at start | 200 KB | 171 KB |

The "after" column includes the Plex edit and Lidarr request features that
landed at the same time, which is why the installer and executable grew a
little; this pass didn't change the native size, and the release profile
was already tuned (below). On the same code as "before", the start bundle
went from 886 KB to 716 KB. Both installers are well under the 15 MB budget.

## Idle wakeups

With the app paused and nothing to do, several loops used to wake it many
times a second. Each now sleeps until there is work.

The player's position tick (4 a second) runs only while a song plays. Five
seconds after playback stops the output stream is paused, so the device
callback (about 100 a second) stops too; the device stays open, so playing
again starts at once. AirPlay keeps streaming, since the receiver keeps time
from it. Decoder threads holding a finished file used to check in every 50
ms and now sleep until a seek, a cancel or the output taking samples wakes
them. The precacher polled every 200 ms and now sleeps when everything it
wants is on disk.

The library watcher polled every second; it now sleeps until a server event,
the quiet period after a change, or the fallback sync while the notification
socket is down. Queued likes and playlist edits were retried every 15 s and
queued scrobbles every 30 s for as long as the app ran; those loops now stop
when their queue is empty and wake when something is queued, and the listen
log backfill stops once it is done. Downloads re-read the library every 30 s
even with nothing downloaded and now wait for a download instead. The sync
file was checked every 3 s with sync off. The music files check looked at
playback problems every 5 s and now does so only around playback, otherwise
once a minute. The network watcher looks every 15 s instead of 5 s. The
Discord party check runs only while the status is on and a song shows, the
"show this computer" recheck only while that switch is on, the new releases
check once a minute, and the update check while offline once a minute
instead of every 10 s.

On Windows the webview isn't told when the window is minimised or hidden to
the tray, so the page kept drawing. The core now tells it, which makes
`document.hidden` true. While hidden every CSS animation is paused and the
lyrics frame loop stops; that loop also stops while a song is paused.

## Window

A position tick re-renders only the seek bar, which a test checks with the
React profiler. The player state sent to the window carries at most the next
500 songs, so playing a whole library no longer sends every song on each
change. Long imported lists and the Missing list draw only the rows near the
viewport, as the Library and Liked Songs already did.

Settings, Library import, the smart playlist editor, the lyrics editor, the
share card, the Plex edit dialogs, Bookmarks, What's New and the miniplayer
load the first time they are opened. Followed artists, listen along and
transcription status load after the first paint. The unused JS opener package was dropped, and the
caches for title widths and artwork colours now have a size limit.

## Already in place

The Rust release profile uses LTO, one codegen unit, `opt-level = 3`,
`panic = "abort"` and stripped symbols. Artwork is requested at its displayed
size times the pixel ratio, and the webview runs with a 64 MB limit on
discardable GPU memory.

## Left to do

Playlist pages still render every row; virtualizing them needs the drag and
selection code to stop finding rows in the DOM. The network watcher could
use the system's network change notifications instead of a timer.
