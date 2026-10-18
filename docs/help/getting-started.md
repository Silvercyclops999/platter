# Getting started

How to sign in, choose your music, pick a look and find your way around Platter.

## Signing in with Plex

Platter plays music from your own Plex server. You sign in through plex.tv in your browser, so Platter never sees your Plex password.

1. Open Platter and press **Sign in with Plex**.
2. Platter opens plex.tv in your browser. Sign in there and approve Platter. If plex.tv asks for a code, type the one Platter shows.
3. Come back to Platter. It finds your server by itself and starts reading your library.

![The Platter sign-in screen](images/sign-in.png)

If the browser didn't open, press **Copy sign-in link** and paste it into your browser yourself. **Cancel** stops the sign-in.

If your account can reach more than one Plex server, Platter asks you to choose the one that has your music. You can add the others later (see [Picking libraries](#picking-libraries)).

Your sign-in is kept in your computer's keychain. To sign out, use **Sign out** in the app menu.

### No Plex account

You can also use a Jellyfin server, or Navidrome or another Subsonic server, without Plex. On the welcome screen, press **Use a Jellyfin server** or **Use Navidrome or another Subsonic server** and follow the steps. Both are in beta. [Connecting to your server](connecting.md#jellyfin-and-subsonic-servers) has the details.

## Your first library sync

The first time, Platter reads your whole library and keeps a copy on your computer. Progress shows at the top of the window, and you can start playing straight away. After that, Platter picks up changes on its own, and a new album on a Plex server shows up within seconds (see [New albums](your-library.md#new-albums)).

If something ever looks out of date, use Settings, Library, Resync library.

## Picking libraries

Your server's main music library is always in. To add more music:

1. Open Settings and go to Library.
2. Under Servers and libraries, turn on the music libraries you want, from your other Plex servers or from servers shared with you.

An album that's on two servers shows once and plays from the best copy. [Connecting to your server](connecting.md#switching-servers) has more about using several servers.

## Picking a look

Right after you sign in for the first time, the Get started guide walks you through your libraries, a look, Discord and online look-ups. You can skip it, and open it again any time from Settings, App, Setup guides. With setup guides turned off, Platter offers **Pick a look** instead.

Studio, Studio Black and Sleeve are Platter's own looks. Studio is warm graphite with a brass accent, Studio Black puts it on true black for OLED screens, and Sleeve is cream paper and ink. Classic is the look Platter had before, and Classic light, Midnight, Forest and Sand are there too. Press **Make your own** to build one with a live preview.

The same window picks a layout too, which decides where the player, your sections and the queue go. [Layouts](layouts.md) describes all six.

You can change the look any time in Settings, Appearance, Theme. Settings, Appearance, Looks has the ready-made looks, your own and theme files. With Settings, Appearance, Follow system light or dark on, Platter switches to a theme's light or dark partner along with your computer: Studio and Studio Black pair with Sleeve, the other themes with Classic or Classic light.

## Home

In Studio, Studio Black and Sleeve, Home starts with records. First comes the album you stopped partway through, with Resume, then New arrivals, Spun this week, Finish what you started and Not played in a year. Mixes, playlists and the other shelves follow. Turn off Settings, Appearance, Home leads with records to get the usual Home back. Classic and the other looks always have the usual Home.

On this day lists what you played on today's date in earlier years, one card for each year. Time travel has a mix of your most played songs from each decade, and one for your year. Both count listening history you imported too. Turn them off with Settings, Appearance, Show On this day on Home and Show Time travel on Home.

## Crates

Crates file your library the way a record shop would: by label, producer, studio, era and genre. Open them from the app menu, or from the link on Home in Platter's own looks. Pick a kind at the top, filter by name, and sort by most records or A to Z. A crate needs two records or more. Its page plays everything in it, and a record that's on the sound map opens there.

Labels come from your files' tags, Plex and the sleeve notes you've opened. Producers come from credit tags and sleeve notes, and studios from sleeve notes looked up online, so crates fill in as you open albums.

Under the songs, an album page shows up to two shelves, such as More from its label and More produced by its producer, from your library only. Other crates it's in are listed under More crates. Turn off Settings, Library, Crate shelves on album pages to leave them out, or Show crates to hide crates everywhere.

## Record shelf

Expand Your Library from the sidebar and it gets one more view: Shelf, in the sort and view menu next to the list and grid views. Your albums stand on shelves as spines in their cover's colour, with the artist and title printed down the side, in whatever order the library is sorted. Artists and playlists stand among them as small tiles.

Point at a spine, or move to it with the arrow keys, and the record comes forward with its cover beside it. Click the spine or press Enter to open the album, use the play button on the cover or press Space to play it, and right-click for the usual album menu. With reduced motion on, the spine lights up instead of moving.

A spine is thicker the more songs the album has, so a double album stands out. Turn off Settings, Library, Spine width follows track count to give every spine the same width. The view you pick and this setting are kept on this computer.

## Discord

Settings, Integrations, Discord, Show what you're playing on Discord is off until you turn it on, in the Get started guide or in Settings. While it's on, music plays and the Discord app is open on this computer, your Discord profile shows the song, artist, album and cover. A private session hides it. [Privacy](privacy.md#discord) explains what goes to Discord.

<!-- TODO: Settings says Discord status isn't available in builds made without a Discord application id. Check whether release builds always have it. -->

## Sleeve notes

An album page has two tabs under its buttons: Songs and Sleeve notes. Sleeve notes is the back of the record: when and where it was recorded, the label, catalogue number, format, edition, country and barcode, the release's own notes, and who made it. Credits are listed for the whole album (produced by, written by, the musicians and what they played, engineered, mixed and mastered by) and then song by song.

Every name is a link. It opens a page with everything in your library that person is credited on, grouped by what they did. A song's menu has **Credits** too, for that one song, in the expanded view as well.

### Where the facts come from

Point at a fact, or move to it with Tab, and small marks show where it came from: **Tags** for your files' own tags, **Plex**, **MB** for MusicBrainz and **Discogs**. A line at the bottom of the tab says what the marks mean. When sources agree, every one is marked. When they disagree, your files' tags win, then Plex, then MusicBrainz, then Discogs.

Settings, Library, Look up credits online asks MusicBrainz (when MusicBrainz is on, Settings, Integrations, Artist info sources) and Discogs (once you've added a Discogs token) when you open the tab. What they say is kept on your computer, so offline you still see it. Unreleased albums are never looked up. Turn off Show sleeve notes to hide the tab and the Credits item.

### Record sides

When an album has a vinyl edition on MusicBrainz or Discogs, its songs are grouped under Side A, Side B and so on (A to D for a double album). Each side's heading has a play button, and the album's menu has Play side A, Play side B, which play just that side. For any other album, Split into sides in the album's menu lets you tick the songs each new side starts on. Your split is kept on every computer you sync, and No sides takes it away. Sides never change the order an album plays in.

Settings, Playback, Pause to flip at the end of a side (off at first) works with the record player in the expanded view: when an album plays from one side into the next, playback stops, the tonearm lifts, and Flip to side B plays on. Turn off Settings, Library, Show vinyl sides to hide sides everywhere.

## First steps

- Press **Home** in the sidebar to see what you played lately and the mixes made for you.
- Search for an artist, album or song with the box at the top, or press `Ctrl+L` (`⌘L` on a Mac). See [Search](search.md).
- Like a song with the plus that shows when you point at it, or with the heart in the player bar. Liked songs gather in Liked Songs in Your Library.
- Play an album and open the queue to see what's next. See [Playing music and the queue](playing-and-queue.md).
- Download an album or playlist to play it without your server. See [Downloads and offline](downloads-and-offline.md).

## Where to find Settings

Click your profile picture (or the first letter of your name) at the top of the window to open the app menu, then choose **Settings**. You can also press `Ctrl+,` (`⌘,` on a Mac). At the top of Settings, Search settings finds a setting by its name or description.

The app menu also has the miniplayer, Private session, Go offline, Check for updates, Keyboard shortcuts and Sign out.

## Getting help

These help pages are built into Platter and work offline. Choose **Help** in the app menu to see them all, or search for anything and press the **Help** chip. Busier screens have a small **?** that opens the right page beside them, and many settings have **Learn more**. Settings, App, Show "?" help buttons hides the **?** buttons.

Settings, App, Setup guides has short guides for getting started, Lidarr, library import, inviting a friend and Stream Deck. Each one picks up where you left it.

Press `Ctrl+/` (`⌘/` on a Mac) for the list of keyboard shortcuts. `Ctrl+K` (`⌘K` on a Mac) opens quick search and commands.

If something isn't working, [Troubleshooting](troubleshooting.md) is a good place to start.
