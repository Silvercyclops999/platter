# Privacy

Short version: your library and what you listen to stay between your computer
and your own Plex server.

## Who the app talks to

- Your Plex server, for browsing, streaming, artwork, and reporting plays and
  progress (so your other Plex apps stay in sync).
- plex.tv, to sign in (a PIN flow in your browser) and to look up your
  server's addresses.
- GitHub, to check for and download app updates.
- Wikipedia and Wikidata, for artist bios and facts (on by default, can be
  turned off in Settings). The request carries the artist's name or
  MusicBrainz id, nothing else.
- Last.fm, Fanart.tv and TheAudioDB, only if you enter your own API key for
  them in Settings. They get the artist's name or MusicBrainz id and your key.
  Their images are fetched from their own image servers.
- ListenBrainz, for an artist's most played songs worldwide (the Popular
  list; on by default, can be turned off in Settings). The request carries
  the artist's MusicBrainz id, and your ListenBrainz user token if you choose
  to enter one.
- MusicBrainz, to find the MusicBrainz id of a featured artist who has no
  albums in your library, when you open their page. The request carries the
  artist's name, nothing else.
- Apple's iTunes Search, only when you copy a public link for a song or album
  from a Share menu (on by default, can be turned off in Settings). The search
  carries the artist and the song or album title, nothing else. Unreleased
  music is never looked up.
- song.link: the public links Platter copies point to song.link pages. The app
  itself never contacts song.link; whoever opens the link does.
- Chromecasts and AirPlay speakers on your own network. While "Show
  Chromecast and AirPlay devices" is on, the app listens for them with
  multicast DNS on the local network (nothing leaves it). When you play on a
  Chromecast, it gets the songs from your Plex server with a short-lived
  token your server hands out for this. When you play on an AirPlay speaker,
  the app sends it the audio.
- Discord, when Discord status is on (it is by default) and the Discord app
  is open on your computer. Platter talks only to that local app, which shows
  the song title, artist, album, how far into the song you are, a cover image
  link and, for released songs, a song.link button on your Discord profile.
  Anyone who can see your profile sees these. Nothing is sent during a
  private session, while paused (unless you choose to show paused songs), or
  when the status is off. Your Plex token and server address are never part
  of it.
- Cover Art Archive, for that cover image: it gets the album's MusicBrainz
  release id, nothing else. Can be turned off in Settings. Discord then loads
  the image from Cover Art Archive itself.
- Imgur, only if you enter your own Imgur client ID in Settings. For albums
  Cover Art Archive doesn't have, the 512 px cover image is uploaded once,
  with your client ID. Nothing else about the song goes with it. Uploaded
  images are public to anyone who has the link, so unreleased covers can be
  kept off Discord (and out of uploads) in Settings. Platter doesn't delete
  uploads.
- LRCLIB (lrclib.net), only if you turn on "Lyrics from LRCLIB when Plex has
  none" in Settings (it's off by default). For a song your Plex server has no
  lyrics for, it gets the artist, title, album and length. If that finds
  nothing, a search sends the artist and title only. This applies to every
  song without lyrics, unreleased ones included. Answers are kept on your
  computer for 30 days (7 days when there were none).

- Hugging Face (huggingface.co), only if you turn on "Transcribe songs
  without lyrics on this computer" and download a speech model. The model
  file is downloaded once from the whisper.cpp model repository there. The
  request carries nothing about you or your music.

Lyrics transcription runs entirely on your computer: songs are read from your
own server (or the app's cache), turned into text on this machine, and saved
in the app's data folder. No audio or text from it is sent anywhere.

Some optional features planned for later will talk to other services, but
only once you turn them on. Examples are Last.fm or ListenBrainz
scrobbling, Lidarr and Tautulli. Each one is off until you set it up, and
this file will list exactly what each one sends.

## What stays on your machine

Your Plex token is kept in the OS keychain (Windows Credential Manager or
macOS Keychain). It's never written to a plain file or to the logs. API keys
and the ListenBrainz token you enter for artist info sources, AirPlay speaker passwords and the Imgur
client ID are kept the same way.

The library cache, artwork cache, search history and listen log live in the
app's data folder, as do the cover links found for Discord and lyrics found on LRCLIB.

## What is never collected

There's no analytics, telemetry or crash reporting that leaves your machine.
If crash reports get added later, they'll be saved locally and only sent if
you agree, and this file will say exactly what's in them.
