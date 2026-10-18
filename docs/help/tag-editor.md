# Tag editor

How to set up the music folder, edit tags in your files, check your tags for problems, and see how Plex reads them.

## Set up the music folder

The tag editor is the only part of Platter that writes to your music files. It edits them in place, so this computer needs to see your server's music folder.

1. Open Settings, Library, Tags and archive, Music folder.
2. Choose Choose… and pick the folder, or type a path like `\\server\music` on Windows or `/Volumes/Music` on a Mac and choose Use this folder.
3. Platter looks for 5 songs where Plex says they are, and says something like "Connected, 5 of 5 files found".

Until the music folder is set, tag details are view only. The setting is for this computer only.

## Edit tags

Choose Edit tags… in an album's or a song's menu. You can change the album fields (album artist, album, year, genre, release type, edition, group, era, quality and explicit) and, for each song, track and disc numbers, title, artist, release status, version note, date, source, flags, explicit and ISRC. When songs differ, a field shows "(several)" and stays as each file has it unless you change it.

Only the fields you change are written. Every other tag, picture and the audio itself stay as they were. Platter never renames or moves a file. Release status goes where the album already keeps it; see [Archive and release status](archive-and-release-status.md#release-status-tag).

To mark a whole album, choose Mark as explicit… or Mark as clean… in its menu. Era pages have the same for an era.

## Saving

Before saving, Platter reads every file again. If one changed since you opened the editor, or another program changed a file in that folder in the last minute, nothing is saved. Each file is written to a checked copy first, so a failure leaves the original as it was.

After a save, a message says how many files were saved, with Undo for a few seconds. Undo puts back what the edit replaced on every file that hasn't changed since.

### Rescan

With Settings, Library, Tags and archive, Rescan in Plex after saving on, Plex rescans just the folders you changed, so it shows the new tags.

### Ask before many files

With Ask before saving more than 50 files on, a save that big tells you how many files it will change and asks first.

## Check tags

Choose Check tags in Settings, Library, Tags and archive to check the whole library, or Check tags… in an artist's or album's menu. Findings are grouped as Needs fixing, Worth fixing and Good to know. Each one says what's wrong and why it matters. Choose Fix to open the editor at that field (this needs the music folder), or This is fine to stop showing it.

It looks for things like:

- a missing album artist
- a year, album name or album artist that differs within an album
- a release type missing its first part
- one folder that Plex shows as two albums
- missing disc numbers on albums with more than one disc
- a release status missing where the album's other songs have one
- repeated or missing track numbers, and file names that don't match the album's pattern

### One-click fixes

Some findings offer a fix: Fill disc tags, Fill release status, Look up ISRCs (from MusicBrainz, then Deezer) and Mark explicit. Each lists every change it would make, and what it leaves alone, before anything is written. Fill release status starts with nothing ticked, because albums often mix statuses on purpose. Fixes save through the editor, so you can undo them.

## Plex library settings

Settings, Library, Tags and archive, Plex library settings shows, for each music library, how Plex treats your tags:

- Local metadata preferred, so Plex doesn't replace your tags with its own matches.
- Embedded tags read.
- Personal Media agent available, for albums MusicBrainz doesn't know, so a wrong match never overwrites an unreleased album.

Anything that's off comes with the steps to change it in Plex. The list is read only.
