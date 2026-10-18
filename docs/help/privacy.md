# Privacy

What Platter sends, where and when, which features stay off until you turn them on, and what stays on your computer.

## The short version

Your library and what you listen to stay between your computer and your own server. Platter has no analytics and no crash reports that leave your computer. Apart from your server, plex.tv and GitHub, outside services have their own switches in Settings.

## Always used

- Your own servers, to browse, stream, show artwork and report plays (so your other Plex apps stay in step). Turn off Settings, Playback, Report plays to your server if you'd rather not.
- plex.tv, to sign in, find your server's addresses, and list your Plex players and Home users.
- GitHub, to check for and download app updates.

## On until you turn them off

- Finding Chromecast, AirPlay and DLNA speakers on your home network (Settings, Downloads and devices, Show these devices), so they show under Listen on. Nothing about you is sent; Platter only asks your network what's there.

Everything else that sends something off your computer, writes to your server or opens a network port starts off. The Get started guide, the other setup guides and Settings turn each one on.

## Off until you turn them on

- Artist bios and facts from Wikipedia and Wikidata, Popular songs and listener numbers from ListenBrainz, and release dates and credits from MusicBrainz (Settings, Integrations, Artist info sources). They get an artist's or album's name or MusicBrainz id, and MusicBrainz never looks up unreleased albums. The Get started guide turns these on with one switch.
- Credits for sleeve notes (Settings, Library, Look up credits online). It's on, but asks nothing until MusicBrainz is on or you add a Discogs token. When you open an album's Sleeve notes, MusicBrainz gets the album's MusicBrainz release id, or its artist and title when there's none. With a Discogs token, Discogs gets the artist and album title and then the release id it found, with your token. Unreleased albums are never looked up.
- Music that isn't in your library, in search, Fans also like and New to you (all under Settings, Library), and the release radar. MusicBrainz and ListenBrainz get only what you typed, or artists' MusicBrainz ids. With Lidarr set up, what you typed goes to your Lidarr instead for artists and albums. Photos of artists you don't have come from Lidarr, Wikimedia Commons or TheAudioDB's free tier, which get only the artist's MusicBrainz id.
- Public share links (Settings, Sharing and friends, Public links for released music). When you copy one, iTunes Search gets the artist and title. Unreleased music never gets one.
- Show this computer to my other devices (Settings, Downloads and devices), which opens a network port so your own computers can play here.
- Keep my queue on the server, and Share my activity with Platter friends, which write to your server.

- Lyrics from LRCLIB, which gets the song's artist, title, album and length. See [Lyrics and transcription](lyrics-and-transcription.md).
- Transcribing lyrics on your computer, which downloads a speech model once from Hugging Face. Your music is never sent.
- Scrobbling to Last.fm or ListenBrainz, with unreleased music left out unless you say so.
- Identify songs by sound, which sends AcoustID a fingerprint and the song's length, and nothing else.
- Last.fm, Fanart.tv and TheAudioDB artist info, with your own key.
- Prompted playlists through your own Anthropic key. Nothing is sent until you press Make rules, and unreleased music is never sent.
- A translation service you set yourself, used only when you press Translate.
- Listening sessions, which go through an encrypted relay so nobody learns anyone's address. See [Listening together](listening-together.md).
- Sync with a folder, which writes one file to a folder you pick and never holds your sign-in or keys.
- Lidarr (see [Lidarr](lidarr.md)), and Tautulli, which Platter only reads your history from.
- The now-playing feed for widgets, automation and Stream Deck. These answer only programs on this computer. See [Automation](automation.md) and [Stream Deck](stream-deck.md).
- Settings, Downloads and devices, Let other Plex apps play on this computer, which opens a network port.

### Discord

Settings, Integrations, Discord, Show what you're playing on Discord talks only to the Discord app on your computer. Your profile then shows the song, artist, album and cover to anyone who can see it. Covers come from Cover Art Archive, which gets only the album's MusicBrainz id. Nothing is sent during a private session or when it's off. Uploading covers to Imgur or a free image host is off until you turn it on, and uploaded covers are public.

## Keys kept in the keychain

Your Plex sign-in is kept in your computer's keychain (Windows Credential Manager or the macOS Keychain), never in a plain file or the logs. So are the API keys you enter, the ListenBrainz token, Last.fm keys, the Imgur client ID, AirPlay passwords, your Anthropic, Lidarr and Tautulli keys, and the sign-ins for Jellyfin and Subsonic servers. On a Mac they share one keychain item, so macOS asks for permission at most once.

Settings, App, Sync, Copy keys from another device moves keys between your computers directly over your home network, never through the sync file.

## What stays on your computer

Your library copy, artwork, search history, listening history and downloads live in Platter's data folder, with a separate one for each Plex Home user.

## Private session and Go offline

A private session, from the app menu or Settings, Sharing and friends, Private session, keeps plays out of your listening history on this device, and stops Discord status, scrobbling and showing what you're playing to friends. Plex still counts finished plays unless Report plays to your server is off.

Go offline, in the app menu, stops all network use until you turn it off. See [Downloads and offline](downloads-and-offline.md#go-offline).

The full list, with every detail, is in [Platter's privacy file](https://github.com/Silvercyclops999/platter/blob/main/PRIVACY.md).
