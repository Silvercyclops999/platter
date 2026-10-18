# Library import

How to bring your playlists, liked songs, saved albums and listening history from other services into Platter, and what the import report means.

## What an import does

Platter reads a file you download from another service and matches every song in it to your library. You see how much you already have, can play imported playlists, and get a list of what's missing.

The files are read on this computer. Nothing changes in Plex unless you choose to save a playlist or like songs there. What you import is synced to your other devices.

## Turning it on

1. Open Settings, Integrations, Library import.
2. Turn on the service you're coming from. Each one is off until you turn it on.
3. Choose **Choose file**, or **Choose folder** if the download is a folder of files. Zip files work too.

Platter says "Importing and matching to your library…" while it works, then shows a short report under the service with **See report**.

## Which services

### Streaming account download

The first service in the list lets you ask for a copy of your data.
<!-- TODO: the help rules don't allow naming this service; the Settings row shows its name -->

1. On the service's website, open Account, then Privacy settings.
2. Tick "Account data" (your library and playlists) and "Extended streaming history" (everything you ever played).
3. Request them. The emails usually take a few days, and up to 30.
4. Pick the zip files from the emails in Platter.

Plays from a private session and podcasts are left out. Only the song, when it played and for how long are kept.

### Tidal, Deezer, Amazon Music, SoundCloud

1. Use a free transfer tool such as TuneMyMusic or Soundiiz to export your library to a CSV file.
2. Pick that file in Platter.

The transfer tool sees your library while it makes the file.

### CSV or M3U list

Pick any CSV or M3U file with at least artist and title columns.

## The import report

Open the report with **See report** or from the Library import page. Each service has a card:

- "812 of 900 songs found" is how many songs from the files Platter matched to songs in your library. The bar shows the same as a percentage.
- "To check" counts songs Platter thinks you have but isn't sure about.
- "Not in your library" counts songs it couldn't find.
- Liked, playlists, saved albums and plays show how many came in, and how many of the liked songs and saved albums you have.
- "New plays added" counts plays added to your listening history, so they show in your [stats](stats-and-wrapped.md). Plays already imported aren't counted twice.
- "Left out" lists what was skipped on purpose: private plays, podcast episodes, plays under 30 seconds, rows without an artist or title, and other files Platter couldn't read.

Songs are matched by their recording code (ISRC) when the file has one, then by names and length, then by names alone.

## Check these matches

Songs Platter isn't sure about wait under **Check these matches**. For each one it shows the library songs it could be. Choose **Use this one** for the right song, or say none of them are it. Your answer is kept when you import again.

## Imported playlists

Imported playlists and liked songs are listed with how many of their songs are in your library. Open one to play the songs you have. Songs you don't have are marked "Not in your library" and can't play. From a playlist's menu you can **Save as playlist** (a new Plex playlist with the songs you have) or **Like matched songs**. Both ask first.

## Missing songs

**See missing songs** lists songs from your imported playlists and likes that aren't in your library, the most wanted first. Each has links to search Bandcamp, Qobuz or iTunes, and **Request** if you use [Lidarr](lidarr.md).

## Played a lot

The Missing page has a second tab, Played a lot. It lists songs and albums your imported listening history played that aren't in your library, the most played first. Plays count when they lasted 30 seconds or more. Switch between songs and albums, and narrow the list by period (All time, Last year or Last 90 days) and by fewest plays (5 to begin with). Hide a row, or a whole artist, if you never want it.

With [Lidarr](lidarr.md) set up, Request on a row asks for that album (a song asks for its album), after the usual safety check. Request these sends up to 10 at once after one confirm. After every library sync, plays of songs you now have are matched to them, so those rows leave the list and count in your stats.

Turn the tab off with Settings, Integrations, Library import, Show "Played a lot" on the Missing page. The same row has the period and fewest plays.

## Bring in my listening

Bring in my listening asks Lidarr for the albums on Played a lot by itself, in waves, the most played first and a few a day. Each album is checked on MusicBrainz and sent the same way as a request you make by hand. In practice mode it only lists what it would ask for.

It pauses while Lidarr's queue is full, while space runs low where Lidarr downloads, while Lidarr can't be reached or its check fails, and after three failed requests in a row. It carries on once that clears. Albums it can't sort out wait under Needs you on Played a lot, with the reason. A card on Played a lot shows how far it's got, and you can pause it there.

Turn it on in Settings, Integrations, Library import, under the Played a lot row. Albums a day, Pause below free space, Pause above Lidarr's queue and Waves (plays) set the pace. Requests and pauses go to the Changes log in Settings, Integrations, Lidarr, and a notification sums up each day.

Bring in my listening runs while Platter is open. With a [Platter Bridge](https://github.com/Silvercyclops999/platter/blob/main/docs/BRIDGE.md) paired, your computer hands the waves to the Bridge, so they carry on while your computers are off, and the card says "Running on your Bridge". The Bridge gets only the albums still to ask for and how many plays each has, never your listening history.

## Importing again or removing

Importing from the same service again updates what's there instead of adding a second copy. Playlists with the same name are replaced and your decisions stay. **Remove imported data** forgets everything from that service, plays included.
