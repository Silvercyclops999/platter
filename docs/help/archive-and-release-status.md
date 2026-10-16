# Archive and release status

How Platter tells released music from unreleased songs, leaks and other versions, and how it treats unreleased music everywhere else in the app.

## How Platter decides

Platter reads the tags your files already have. You don't have to retag anything. The first of these with a value it knows decides:

1. Platter's own `ARCHIVE_VERSION` tag, like `demo`, `leak` or `released`.
2. A plain `STATUS` tag, like `Released`, `Unreleased` or `Leaked`.
3. The release status MusicBrainz Picard writes, like `official` or `bootleg`.
4. A release type with a `demo` part, which makes the song an alternate version.

A value Platter doesn't know never counts as released. An album follows most of its songs on disc 1, and each song keeps its own status.

Albums Plex couldn't match usually have no release type and no label. Settings, Library, File tags, Count albums Plex couldn't match as unreleased treats them as unreleased until you tag them, and Check MusicBrainz for albums Plex couldn't match (off until you turn it on, since it asks MusicBrainz) counts them as released when MusicBrainz knows them.

## Labels and Why?

Album pages can show Probably unreleased, Unreleased, Released, Plex missed it, or Marked released. Choose Why? to see every piece of evidence behind it. Then choose Confirm or Not unreleased to settle it. Your answer syncs to your other devices and your files aren't changed. A song's own answer beats its album's, and the file's tags beat both.

## Release status tag

If your library keeps release status in a tag of its own, open Settings, Library, Archive, Release status tag, choose Custom tag and type its name, like `PROVENANCE`. Platter reads it after `ARCHIVE_VERSION` and reads your files again in the background. Auto goes back to the order above.

Under Release status values, type a value your tags use, pick what it Means and choose Add. Common values like Official, Leak, Alt and OG file are known already.

To change the tags themselves, use the [tag editor](tag-editor.md).

## Folder keywords

Settings, Library, Archive, Folder names that hint at unreleased music starts with Unreleased, Leaks, Demos, Sessions and Snippets. When a song's folder has one of these names, Why? lists it for albums that are probably unreleased. A folder name adds weight but never decides on its own. You can keep up to 20. Empty the list to stop using folder names.

## Versions and lineage

Released songs with demos, leaks or other versions in your library show +N versions next to the title (Show other versions on songs). Other versions is in the song's menu either way.

With Show lineage on, Lineage puts every version on one timeline, from the first demo to the live takes. Compare switches between two versions at the same moment. Compare versions at the same loudness plays both at the same loudness when your server has the data.

## Eras

Tag songs with `ARCHIVE_ERA`, and `ARCHIVE_DATE` as `2019`, `2019-05` or `2019-05-31`. Artists with two or more eras get era cards under Popular (Show era cards on artist pages). Pick a card to show only that era, released and unreleased, then Open the era page to see each release in the era, newest build first, with play and shuffle.

Artist pages list unreleased albums under Unreleased, and Move to section puts an album in a section of your own.

## How unreleased music is treated

- Discord: Settings, Integrations, Discord has Show unreleased songs, Show unreleased cover art, and Upload covers for unreleased songs (off at first). Unreleased songs never get a Find this song button.
- Sharing: unreleased music never gets a public song.link page, and share cards and monthly recap cards leave it off. Copy Platter link still works.
- Mixes: Settings, Mixes, Include unreleased music decides whether demos and other versions go in mixes and radio.
- Scrobbling: Settings, Integrations, Scrobbling, Send plays of unreleased music is off at first, so those plays stay on this computer.
- [Listening sessions](listening-together.md): unreleased songs only play for people with access to your library, and you can hide their titles.
- [Lyrics](lyrics-and-transcription.md): unreleased songs can be transcribed in the background, and aren't offered for public sharing unless you allow it.
