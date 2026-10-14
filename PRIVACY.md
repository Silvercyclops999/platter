# Privacy

Short version: your library and what you listen to stay between your computer
and your own Plex server.

## Who the app talks to

- Your Plex server, for browsing, streaming, artwork, and reporting plays and
  progress (so your other Plex apps stay in sync).
- plex.tv, to sign in (a PIN flow in your browser) and to look up your
  server's addresses.
- GitHub, to check for and download app updates.

Some optional features planned for later will talk to other services, but
only once you turn them on. Examples are Discord status, an image host for
Discord cover art, Last.fm or ListenBrainz scrobbling, Lidarr and Tautulli.
Each one is off until you set it up, and this file will list exactly what
each one sends.

## What stays on your machine

Your Plex token is kept in the OS keychain (Windows Credential Manager or
macOS Keychain). It's never written to a plain file or to the logs.

The library cache, artwork cache, search history and listen log live in the
app's data folder.

## What is never collected

There's no analytics, telemetry or crash reporting that leaves your machine.
If crash reports get added later, they'll be saved locally and only sent if
you agree, and this file will say exactly what's in them.
