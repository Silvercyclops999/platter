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

Some optional features planned for later will talk to other services, but
only once you turn them on. Examples are Discord status, an image host for
Discord cover art, Last.fm or ListenBrainz scrobbling, Lidarr and Tautulli.
Each one is off until you set it up, and this file will list exactly what
each one sends.

## What stays on your machine

Your Plex token is kept in the OS keychain (Windows Credential Manager or
macOS Keychain). It's never written to a plain file or to the logs. API keys
you enter for artist info sources, and AirPlay speaker passwords, are kept
the same way.

The library cache, artwork cache, search history and listen log live in the
app's data folder.

## What is never collected

There's no analytics, telemetry or crash reporting that leaves your machine.
If crash reports get added later, they'll be saved locally and only sent if
you agree, and this file will say exactly what's in them.
