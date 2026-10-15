# Platter

A desktop music player for Plex libraries, for Windows and macOS. Playback,
search and the library cache run in Rust, so browsing is fast and your files
play at their original quality.

It's early. The core player is still being built and it isn't ready for
everyday use yet.

Platter is an independent project and isn't affiliated with Plex, Inc. or
Spotify. See [TRADEMARKS.md](TRADEMARKS.md).

I listen to everything off my own Plex server. Plexamp is good, but I wanted
something that feels more like the big streaming apps: search that keeps up
with typing, quick browsing, a proper queue, mixes, a recap at the end of the
year. And I keep a lot of unreleased and alternate versions of albums, which
no Plex app really has a place for.

So the plan is a player that treats your server like a streaming catalogue,
plus an archive section for the stuff that never got an official release.

## Features

These are the ones being built right now:

- Sign in with Plex in your browser. The app never sees your password, and
  the token is stored in the OS keychain (Windows Credential Manager or macOS
  Keychain).
- Finds your server through your Plex account and connects to it directly.
  It tries your home network first, then Plex remote access, then the relay.
  You don't need a VPN.
- Copies your music library into a local cache, so browsing and search are
  quick and still work offline.
- Plays your original files (FLAC, ALAC, MP3, AAC, WAV and so on) with a
  queue that works the way you'd expect. The engine is set up for gapless
  playback, which comes next.
- Reports plays and progress back to Plex, so Plexamp and Plex Web stay in
  sync.
- When the server or network drops out it tells you what's wrong in plain
  words and keeps playing whatever it still can.
- Updates itself from GitHub releases.

## Roadmap

Right now I'm working on the core player: sign-in, the library, playback, the
queue, search, offline states and auto-update.

After that comes gapless playback and crossfade, loudness leveling, an EQ,
likes, playlists, media keys and a Discord status.

Later on I want mixes and radio built from your own library, downloads for
offline listening, handing playback between PC and Mac, lyrics, a yearly
recap, and the archive view (eras, versions and editions of an album). Mobile
might happen after 1.0.

## Screenshots

I'll add screenshots as screens get finished. They come from the app's
built-in demo library (made-up artists and generated artwork), never from a
real server.

## What it isn't

It only plays music, and only from Plex servers you can reach. There's no
outside catalogue, so recommendations come from your own library.

Nothing gets installed on your server. No container, no companion service.
Plex is the only backend.

It's also one person's project, built around how I listen. There will be bugs
I haven't run into yet.

## Privacy

Platter talks to your Plex server and to plex.tv (for sign-in and to find
your server), and that's it. There's no analytics or crash reporting that
leaves your machine. Details are in [PRIVACY.md](PRIVACY.md).

## Building it

1. Install Git, Node.js 22+, and Rust (<https://rustup.rs>).
2. On Windows, install Visual Studio Build Tools with "Desktop development
   with C++" (WebView2 already ships with Windows 11). On macOS, run
   `xcode-select --install`.
3. `npm install`
4. `npm run tauri dev`

Discord status needs a Discord application id, read when the app is
compiled. Create an application in the Discord Developer Portal and set
`PLATTER_DISCORD_APP_ID` to its id before building, for example
`PLATTER_DISCORD_APP_ID=<your app id> npm run tauri build`. Without it the
build works the same, and Settings says Discord status isn't available.

Lyrics transcription (whisper.cpp) is the `transcribe` Cargo feature, which
release builds turn on: `npm run tauri build -- --features transcribe`. It
needs CMake, and on Windows also LLVM (for libclang). On macOS and Linux,
setting `WHISPER_DONT_GENERATE_BINDINGS=1` skips the libclang step. Without
the feature everything else builds the same, and Settings says transcription
isn't in this build.

Run the tests with `npm test`, and `cargo clippy -- -D warnings && cargo test`
inside `src-tauri/`. There's a bit more in [CONTRIBUTING.md](CONTRIBUTING.md).

```
docs/            Plex API notes, tagging rules, IPC contract
src/             React + TypeScript interface
src-tauri/       Rust core: audio, Plex client, cache + search, platform glue
streamdeck/      Stream Deck plugin, built on its own (see its README)
```

## How it's made

I'm building Platter with a lot of help from AI (Claude Code). Other apps are
only a reference for how things should behave. No code, assets or text are
taken from them.

## Contributing

Issues and pull requests are welcome, and I'd especially like to hear from
people whose library or setup looks different from mine. Ideas can go in
Discussions. Please report security issues through [SECURITY.md](SECURITY.md)
rather than a public issue.

## Licence

GPL-3.0-or-later. The Platter name and brand artwork aren't covered by that
licence; see [TRADEMARKS.md](TRADEMARKS.md). "Plex" and "Spotify" are
trademarks of their respective owners.
