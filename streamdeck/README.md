# Platter for Stream Deck

A Stream Deck plugin that controls Platter on the same computer, with keys, Stream Deck + dials and the Stream Deck Neo info bar. Every action is in one Platter category, and each has its own settings panel in the Stream Deck app (where you can also give a key your own picture for each of its states).

It talks only to Platter's control API on 127.0.0.1, which is off until you turn on Allow Stream Deck in Platter's Settings. Nothing goes over your network.

Needs Stream Deck 7.6 or newer.

## Installing

The plugin comes with Platter on Windows and macOS. In Platter, open Settings, find Stream Deck and click Install for Stream Deck. That turns on Allow Stream Deck and hands the plugin to the Stream Deck app, which installs it. When a newer Platter brings a newer plugin, the same button says Update.

Without the Stream Deck app, Settings links to Elgato's download page instead. Each release also has `app.platter.streamdeck.streamDeckPlugin` as a download; opening it installs the plugin too.

## Actions

**Playback**

- **Play / Pause**: the cover fills the key, with no text. Paused, a see-through circle with a play symbol sits over it (or a small Paused label). Press twice for next, three times or hold for previous; in a multi-action a press just plays or pauses. Options: the time (elapsed or remaining) in a band at the bottom, the title, and big art: Play / Pause keys side by side (2 x 2, 3 x 3 or 4 x 4) share one large cover, found by themselves or set by hand.
- **Previous**, **Next**: optionally with the cover of the song before or after.
- **Seek back**, **Seek forward**: 10 seconds by default; hold to keep going.
- **Song info**: title and artist over the dimmed cover, the elapsed time and the song's length, and a bar, updated every second. Press to open now playing in Platter. It can also write the song to the clipboard or a text file (a format with {title}, {artist}, {album}, {elapsed}, {total}), rewritten on every song change, for streaming overlays.

**Modes**

- **Shuffle** (hold for smart shuffle), **Repeat** (off, all, one), **Crossfade**, **Private session**: each shows whether it's on.

**Volume**

- **Volume up**, **Volume down**: step 1 to 25 (5 by default), hold to repeat, optionally the level as a bar or a number.
- **Mute**: mutes and unmutes, keeping the level. **Volume set**: an exact level.
- **Multimedia** dial: the player view (title and artist, elapsed time on the left, the song's length on the right, a thin bar; long titles fade at the edge). Turn for volume (or seek, or skip songs), with an overlay that hides two seconds after you stop. Press to play or pause, touch to play or pause, press twice to switch to the browser (playlists, albums or mixes with their covers, or a list): turn to move, press to play.
- **Play / Pause** dial (turn to seek) and **Volume** dial (press to mute).

**Library**

- **Like**: shows whether the song is liked.
- **Add to playlist**: a playlist, shown with its cover and name, or Ask (Platter comes up with its playlist picker). Pressing again on the same song takes it out.
- **Remove from playlist**: greyed when the song isn't in it.
- **Rate**: 1 to 5 stars (Plex); pressing the rating the song has clears it.

**Play things**

- **Play playlist**, **Play album**, **Play artist**, **Play mix**: pick one from your library (the key shows its cover), shuffled or not, played now or added to the queue, optionally from a time into the first song.
- **Play link** (a platter:// link from Share), **Play search** (the top songs for a search), **Song radio**, **Daylist**, **Discover**, **Liked Songs**.

**Queue and devices**

- **Queue**: the next song's cover with "Next: title"; press to open the queue. The **Queue** dial turns through what's coming up; press to jump to it.
- **Listen on**: a device from Platter's Listen on list (the key shows its name), or Ask. The **Listen on** dial turns through the devices.

**Platter**

- **Lyrics**, **Miniplayer**, **Show / hide Platter**: each shows whether it's open.
- **Sleep timer**: minutes, or the end of the song. One timer for all keys; the key counts down. Press to stop it (or to add time, a setting); hold to start again.
- **Bookmark**: bookmarks this moment of the song.
- **Request album**: opens Platter's usual Lidarr request for the song's album (once Lidarr is set up). **Listen along**: starts a session in Platter and shows its invite.
- **Now playing** on the Stream Deck Neo info bar.

## What keys show

- Platter not running (no answer for about 8 seconds): keys grey out and say so.
- Nothing playing: Song info and the dials say Nothing playing, with a dash for the artist; keys that need a song grey out.
- Not paired, or Allow Stream Deck off: a small badge over the usual picture. While Platter is asking, keys say Approve in Platter.
- When playback stops, the last song stays up for about 8 seconds.
- A key that can't run right now is greyed with a short reason as its title.

## Pairing

Drag any Platter action onto a key. The first time the plugin reaches Platter it asks to pair: Platter's window comes up with "Allow Stream Deck to control Platter?" and the keys say "Approve in Platter". Choose Allow and every key works; nothing else to set up.

If you choose Don't allow (or leave it), the plugin stops asking until you press one of its keys. Platter shows one prompt at a time and never allows by itself.

The six-digit code still works: in Platter's Settings, under Allow Stream Deck, there's a code to type into any Platter action's settings panel in the Stream Deck app. That panel also has Ask Platter, and Unpair.

The plugin keeps the token Platter gives it, and all keys share it. Revoke in Platter's Settings, or Unpair in the plugin, forgets it.

## Building

Needs Node.js 20 or newer.

```
npm ci
npm run build      # bundles src/ into app.platter.streamdeck.sdPlugin/bin/plugin.js
npm test           # client, pairing, timing, settings, key, dial and picture tests
npm run typecheck
npm run pack       # writes dist/app.platter.streamdeck.streamDeckPlugin
```

Open the `.streamDeckPlugin` file to install it. While working on it, `npx streamdeck link app.platter.streamdeck.sdPlugin` installs the folder itself.

The release build packs the plugin and bundles it into Platter with `--config src-tauri/tauri.streamdeck.conf.json`. To try Install for Stream Deck from a local build, run `npm run pack` here first and build Platter with that same `--config`. Bump `Version` in the manifest (and `version` in package.json) when the plugin changes, so Settings offers the update.

The glyphs are in `scripts/glyphs.mjs`, drawn for Platter. `scripts/icons.mjs` turns them into the action list icons and each key's starting picture; run `node scripts/icons.mjs` after changing one. Live pictures are SVG, made in `src/render.ts`, so they're sharp on every key size; covers come at 288 px.
