# Lyrics and transcription

Where lyrics come from, how Platter checks and fixes them, and how to write or transcribe your own.

![The lyrics view with Not checked, the timing buttons, Tap to sync and Wrong lyrics?](images/lyrics.png)

## Where lyrics come from

Platter looks in this order:

1. Your server: a `.lrc` or `.txt` file next to the song, or lyrics Plex has.
2. Lyrics you wrote in Platter.
3. LRCLIB, a free lyrics library, if you turn it on.
4. A transcription made on this computer, only when nothing else fits.

To show your own lyrics first, turn on Settings, Lyrics, Prefer my own lyrics over Plex's.

### When there are none

If a lyrics source couldn't be reached, the view says Couldn't get lyrics right now. Press **Retry**, or wait: Platter asks again when you're back online. That's different from no lyrics anywhere, and from This song is instrumental, which shows when the title says instrumental or karaoke, or LRCLIB marks the song that way.

### LRCLIB

Settings, Lyrics, Lyrics from LRCLIB when Plex has none starts off. Platter sends the song's artist, title, album and length, and keeps answers for 30 days. On live versions, found lyrics show without timing unless you turn on Show borrowed timing on live versions.

## Checking lyrics against the song

With Settings, Lyrics, Check lyrics against the song on, Platter listens to a short part of the song with the speech model on this computer and skips lyrics that belong to another song. The song never leaves your computer. Until the [speech model](#the-model-download) is downloaded, lyrics show as Not checked.

Lyrics show straight away while the check runs, marked Checking lyrics against the song. If they turn out to belong to another song, Platter moves to a set that matches, or says None of the lyrics found match this song.

### Timing fixes

Settings, Lyrics, Fix lyrics timing automatically moves checked lyrics that run early, late or drift. To move them yourself, use the minus and plus buttons under the lyrics (or the `-` and `+` keys), or Tap to sync. Every quick press counts. While the check is still running, the buttons wait, since it may move the timing itself, and they come back as soon as it's done (or if it gets stuck).

Demos and other versions ask before using the released song's lyrics (Use anyway), unless you turn on Use released lyrics for other versions.

## Wrong lyrics?

Choose Wrong lyrics? under the lyrics or in a song's menu to see every set found and what the check said. Pick another set, or choose Transcribe this song, Write or paste lyrics, or No lyrics for this song. Let Platter pick goes back to automatic. **Look again** asks the lyrics sources again, in case they have the song now.

Your picks and timing win over everything automatic. Settings, Lyrics, Lyrics choices and timing has Reset.

## Writing your own

Choose Add lyrics or Edit lyrics and type or paste the words, one line each. To sync them, play the song and press `Space` as each line starts. Your lyrics stay in Platter, never in your music folder, and Export .lrc saves a copy.

## Translation and pronunciation

Settings, Lyrics, Show pronunciation adds a romanised line under Japanese, Korean, Chinese, Hindi and Cyrillic lyrics, made on this computer.

For translation, put the address of a LibreTranslate-compatible service in Settings, Lyrics, Translation service address, and pick a language in Translate lyrics into. Lyrics then get a Translate button. With no service set, nothing is sent.

## Transcription

Turn on Settings, Lyrics, Transcribe songs without lyrics on this computer. Transcribe this song then shows in the lyrics view, and Transcribe songs without lyrics in an album's menu. Transcribe all unreleased songs without lyrics, with Transcribe now, works through them in the background. The audio never leaves your computer. The models understand English only; fix mistakes in the lyrics editor.

### The model download

The model downloads once, the first time you need it, or now with Download under Speech model. Small (181 MB) is right for most songs. Medium (514 MB) is a little more accurate but much slower. Delete model frees the space.

### Processor and graphics card

On Windows and Linux the model runs on the processor at low priority. On a Mac it uses the graphics card (GPU) when it can, which runs cooler. Processor use sets how hard it works: Light (the default), Balanced or Fast. Transcription needs a processor with AVX2 (most from 2013 on).

### Sharing transcriptions

Sync transcriptions to my devices sends them through your sync file, so each song is transcribed once. It needs Lyrics you wrote on under Sync.

With Offer to share transcriptions publicly on, a transcription you've reviewed gets Share these lyrics publicly. It shows exactly what goes to LRCLIB and asks every time. [Unreleased songs](archive-and-release-status.md) are left out unless you turn on Including unreleased songs, because sharing makes the title public.
