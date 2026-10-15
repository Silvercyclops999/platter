# Privacy

Short version: your library and what you listen to stay between your computer
and your own Plex server.

## Who the app talks to

- Your Plex server, for browsing, streaming, artwork, and reporting plays and
  progress (so your other Plex apps stay in sync).
- Your other Plex servers and servers shared with you, only for the libraries
  you include in Settings (and to list their libraries while Settings is
  open). Plays, likes and playlists for their songs go to the server that
  has the file.
- plex.tv, to sign in (a PIN flow in your browser), to look up your
  server's addresses, to list your other Plex players, to list and switch
  Plex Home users, and to publish this computer's local addresses for your
  other Platter computers (see below).
- GitHub, to check for and download app updates.
- Wikipedia and Wikidata, for artist bios and facts (on by default, can be
  turned off in Settings). The request carries the artist's name or
  MusicBrainz id, nothing else. The artist's website and profile links shown
  in the About dialog come from the same answer and only open, in your
  browser, when you click one.
- Last.fm, Fanart.tv and TheAudioDB, only if you enter your own API key for
  them in Settings. They get the artist's name or MusicBrainz id and your key.
  Their images are fetched from their own image servers.
- ListenBrainz, for an artist's most played songs worldwide (the Popular
  list; on by default, can be turned off in Settings). The request carries
  the artist's MusicBrainz id, and your ListenBrainz user token if you choose
  to enter one.
- ListenBrainz listener numbers for an artist, when Last.fm has none (on by
  default, can be turned off in Settings). The request carries the artist's
  MusicBrainz id.
- MusicBrainz, to find the MusicBrainz id of a featured artist who has no
  albums in your library, when you open their page. The request carries the
  artist's name, nothing else. With MusicBrainz on in Settings (the default),
  it also gives an artist's members and related artists (the request carries
  their MusicBrainz id) and an album's release date and credits (the album's
  MusicBrainz release id, or its artist and title when Plex has no id).
  Unreleased albums are never looked up.
- ntfy.sh (or your own ntfy server), only during a listening session. It
  carries the session's sealed messages; see Listening sessions below.
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
- Your other computers running Platter. While "Show this computer to my
  other devices" is on (it is by default), the app announces itself on your
  local network with multicast DNS. The announcement holds an id for this
  install, the name you give the computer (its own computer name until you
  change it), whether it's Windows or macOS, the app version, and a short
  one-way hash of your Plex account id so your own computers recognise each
  other. It holds no token and nothing about your library or what's playing.
  The app also opens a port on its local network addresses (32520, or the
  next free one up to 32529) so your other Platter computers can start and
  control playback here. A request is only accepted when it's signed with a
  key made from your Plex sign-in, and the request never contains the token
  itself. The app tells plex.tv those local addresses, the way Plex players
  do, so your other computers can find this one through Plex. Turning the
  setting off stops the announcement and closes the port.
- Discord, when Discord status is on (it is by default) and the Discord app
  is open on your computer. Platter talks only to that local app, which shows
  the song title, artist, album, how far into the song you are, a cover image
  link and, for released songs, a song.link button on your Discord profile.
  Anyone who can see your profile sees these. Nothing is sent during a
  private session, while paused (unless you choose to show paused songs), or
  when the status is off. Your Plex token and server address are never part
  of it. After Discord takes a status with a cover, Platter loads Discord's
  copy of that image once from media.discordapp.net (and up to three more
  times over a minute if it isn't ready), to make sure the cover shows. That
  request carries only the image link Discord made.
- Cover Art Archive, for that cover image: it gets the album's MusicBrainz
  release id, or a MusicBrainz release group id, nothing else. Can be turned
  off in Settings. Discord then loads the image from Cover Art Archive itself.
- MusicBrainz, for that same cover when Plex has no MusicBrainz id for a
  released album: it gets the album artist and album title, once per album.
  Unreleased albums are never searched. Off with the Cover Art Archive
  setting.
- Imgur, only if you enter your own Imgur client ID in Settings. For albums
  Cover Art Archive doesn't have, the 512 px cover image is uploaded once,
  with your client ID. Nothing else about the song goes with it. Uploaded
  images are public to anyone who has the link, so unreleased covers can be
  kept off Discord (and out of uploads) in Settings. Platter doesn't delete
  uploads.
- catbox.moe, only if you turn on "Upload covers to catbox.moe" in Settings
  (it's off by default) and Imgur isn't in use. For albums Cover Art Archive
  doesn't have, the 512 px cover image is uploaded once, without an account.
  Nothing else about the song goes with it. The image is public to anyone
  with the link and can't be deleted from Platter. catbox.moe's own rules
  apply to what's uploaded.
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

## Friend activity and listening sessions

Friend activity reads your own Plex server: who is playing music right now
(`/status/sessions`), the music plays of the last week from the server's play
history, and the names on the server's accounts. Plex only answers these for
the server's owner, so for everyone else the panel stays empty. What it shows
(names, profile pictures, songs) stays on your computer and is read again
while the panel is open. It goes nowhere else.

What others see of you is what Plex records. While a song plays, Platter tells
your server about it, and that's what makes you show up live for the server's
owner. Turn off "Share my activity with Platter friends", or start a private
session, and Platter stops doing that. Songs you finish still go into Plex's
play history unless you're in a private session or "Report plays to Plex" is
off, and other Plex apps the owner uses can see that history.

The artists you follow are kept in the app's data folder on this computer,
and in the sync file if you turn that on (see below).

### Listening sessions

People in a listening session never connect to each other. Everyone
connects only to a relay, so nobody in the session, the host included,
learns anyone else's IP address. The relay is ntfy.sh, a free public ntfy
server, unless you set your own ntfy server in Settings.

What the relay sees: random topic names, and messages sealed end to end
(ChaCha20-Poly1305) that it can't read, kept for up to 12 hours on ntfy.sh.
Like any website, the relay's operator also sees the IP address each person
connects from. It never passes one person's address to another. Platter
asks ntfy.sh not to forward the messages to Firebase.

What the others in a session see: the name you choose in Settings (your
Plex name if you leave it empty), the songs playing and in the queue, and
who added each one. No email, location, computer name or picture.

The invite link holds only a random token: a session id, a secret, when it
runs out, and your relay's address if you set your own. It never holds an
IP address, a host name, a port, your Plex server's address or id, or a
token. Each link works for one person, once, and runs out after 24 hours
(you can choose 1 hour or 7 days). Removing someone takes their keys away,
and ending the session takes everyone's.

Guests play the songs from their own access to your Plex server. Platter
never sends your Plex token, your server's address or a stream link, so
someone without access to your library can't listen along. Note that your
server itself sees the address of each guest who streams from it, as it
always does for anyone you share your library with.

Platter's logs never record addresses, tokens, keys or topic names from a
session. While you host a session and Discord status is on, Discord shows
that you're in one and how many people are in it.

## Scrobbling

Scrobbling is off until you turn it on for Last.fm or ListenBrainz in
Settings. When it's on, each song you play past the halfway point (or for 4
minutes) is sent to that service with its title, artist, album, album artist,
track number, length and the time you started it, and each song that starts
is sent as "now playing". That's what shows on your profile there. Unreleased
music is left out unless you turn on "Scrobble unreleased music", and a
private session sends nothing. Plays that can't be sent right away wait on
this computer for up to two weeks.

Last.fm needs your own API key and its shared secret. Signing in opens
Last.fm's site, where you approve Platter; Last.fm then gives Platter a
session key. The key, the secret and the session key stay in your computer's
keychain and are only ever sent to Last.fm. ListenBrainz uses your user
token, kept the same way and only sent to ListenBrainz.

## Prompted playlists and the AI provider

The AI provider is None until you choose one and save its key. With
Anthropic chosen, making a prompted playlist sends Anthropic your
description, the names of your library's most used genres, styles and moods,
and the range of years in it. A second request sends the titles, artists,
albums and years of up to 150 songs that fit, so it can pick and order them.
Unreleased music is never sent, and nothing is sent until you press Make
rules. The key stays in your computer's keychain and is only sent to
Anthropic. Anthropic's own terms cover what it does with requests.

## Hidden songs, bookmarks and smart playlists

Hidden songs and artists, bookmarks and smart playlists kept by Platter are
stored on this computer, and in the sync file when you turn that on. Smart
playlists whose rules Plex can hold are saved in Plex, like any playlist.

Some optional features planned for later will talk to other services, but
only once you turn them on. Examples are Lidarr and Tautulli. Each one is
off until you set it up, and this file will list exactly what each one sends.

## The sync file

Sync with a folder is off until you turn it on in Settings. When it's on,
Platter keeps one small file, `Platter/sync-<tag>.json`, in a folder you pick,
usually one your cloud app (OneDrive, iCloud Drive, Google Drive, Dropbox)
already keeps in step. Platter never uploads it anywhere itself; your cloud
app does that like any other file. The tag is a hash of your Plex account id,
not the id.

The file holds the settings that aren't marked This device only, your pins,
library folders and which playlists are in them, followed artists, hidden and
snoozed items, bookmarks, lyrics you wrote in the editor, and for each of your
devices its name and when it last wrote the file. You choose which of these go
in under What to sync. It never holds your Plex token, API keys, the
ListenBrainz token, the Imgur client ID, AirPlay passwords or any other
secret: the app refuses to put anything shaped like one in, and a test checks
the written file.

To find a folder to suggest, Platter looks for the usual OneDrive, iCloud
Drive, Google Drive and Dropbox folders on this computer. It only looks; it
writes nothing until you pick one.

Copying keys between your computers (Settings, Sync) never uses the file.
The two computers talk directly on your local network, both signed in to the
same Plex account. The 6-digit code shown on one is typed on the other; it
turns into a one-time key, and the keys you tick are sent sealed with it and
go straight into the receiving computer's keychain. The code and the keys are
never logged.

## What stays on your machine

Your Plex token is kept in the OS keychain (Windows Credential Manager or
macOS Keychain). It's never written to a plain file or to the logs. API keys
and the ListenBrainz token you enter for artist info sources, AirPlay speaker passwords and the Imgur
client ID are kept the same way.

The library cache, artwork cache, search history and listen log live in the
app's data folder, as do the cover links found for Discord and lyrics found on LRCLIB.

The now-playing feed for widgets is off until you turn it on in Settings. It
opens a port on 127.0.0.1 only (32540 unless you pick another), which only
programs on this computer can reach, never other devices on your network. It
serves the playing song's title, artists, album, cover, position and length,
and nothing else: no token, no server address, no library. It answers only
requests addressed to 127.0.0.1 or localhost and gives web pages no way to
read it from another site. A private session blanks it, and turning it off
closes the port.

If your Plex account is a Plex Home, each Home user you switch to in Platter
gets their own folder inside the data folder and their own keychain entries,
so their library, likes, history, queue, downloads and settings stay apart.
Switching asks plex.tv for that user's token. A user's PIN is typed by you,
sent once to plex.tv to switch and never saved.

Downloads are copies of songs from your own Plex server, kept in the app's
data folder (or a folder you pick) with their album covers. They never go
anywhere else, and Remove all downloads in Settings deletes them.

Go offline (in the app menu) stops network use: nothing goes to your
server, plex.tv, GitHub or the services above until you turn it off. The one
exception is a speech model download you start yourself in Settings.

## What is never collected

There's no analytics, telemetry or crash reporting that leaves your machine.
If crash reports get added later, they'll be saved locally and only sent if
you agree, and this file will say exactly what's in them.
