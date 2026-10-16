# Automation

How to control Platter from links, scripts, the command line, Home Assistant and Raycast.

## Turning it on

Everything on this page is off until you turn it on.

1. Open Settings, Automation.
2. Turn on **Allow automation**.

It stays on only for this computer. Nothing listens on your network: only programs on this computer can reach Platter, and web pages can't use it. Under the switch you'll see a six-digit pairing code, which scripts and the command line use once to pair.

## Links

With automation on, opening a `platter://` link controls the running app. You can put links in a shortcut, a launcher or a script.

- `platter://play`, `platter://pause` and `platter://toggle` play, pause, or switch between the two.
- `platter://next` and `platter://previous` skip.
- `platter://like` likes the playing song, or unlikes it if it's already liked.
- `platter://play?playlist=Road%20Trip` plays a playlist. Add `&shuffle=1` to shuffle it.
- `platter://search?q=night%20drive` searches.
- `platter://open/liked` opens a page. It knows home, search, library, liked, settings, stats, recents, bookmarks, lyrics, release-radar and whats-new.

A playlist can be named by its name. An exact match wins, then a playlist whose name starts with what you typed. Links that play or skip leave the window where it is; search and open bring it to the front.

Links don't need pairing, since only something already on your computer can open them. If automation is off, Platter says so and does nothing.

## Command line

Platter's own program doubles as a small command line. It talks to the copy that's already running and never starts the app.

1. Find the program. On Windows it's `Platter.exe` in the install folder. On macOS it's inside the app at `Platter.app/Contents/MacOS/Platter`.
2. Pair once with the code from Settings, for example `platter --pair 123456`.
3. Then use `--play`, `--pause`, `--toggle`, `--next`, `--previous`, `--like`, or `--playlist "Road Trip" --shuffle`.

`--now-playing` prints the song and artist. Add `--json` to get the title, artist, album, whether it's playing and liked, and the volume. During a private session the song is left out.

When it finishes, the command returns 0 if it worked, 1 if Platter said no (the message says why), 2 if the command was typed wrong, and 3 if Platter isn't running or automation is off.

## Other programs

Any program on this computer can use the same connection the command line uses, on port 32541. It pairs by sending the code once and gets a token back to send with each request. The details are in Platter's [developer notes on automation](https://github.com/Silvercyclops999/platter/blob/main/docs/AUTOMATION.md).

Pairing a script replaces any script paired before. The Stream Deck has its own pairing and isn't affected (see [Stream Deck](stream-deck.md)).

## Home Assistant

Home Assistant usually runs on another machine, and Platter only answers this computer, so you need a helper on the computer running Platter. A companion app that can run commands on your computer, such as HASS.Agent on Windows, works well.

1. In the helper, add custom commands that run `platter --toggle`, `platter --next` and so on.
2. They show up in Home Assistant as buttons for a dashboard or your automations.
3. To show what's playing, have the helper run `platter --now-playing --json` as a sensor.

## Raycast

Raycast script commands are small shell scripts with a short header. Make one per action:

```
#!/bin/bash
# @raycast.schemaVersion 1
# @raycast.title Next song
# @raycast.mode silent
open "platter://next"
```

Use `platter --now-playing` with `@raycast.mode inline` to show the playing song in Raycast.

## Your own scripts

From a shell, the command line is simplest. A script that pauses music when a call starts might run `platter --pause`, then `platter --play` afterwards. In PowerShell, `Start-Process "platter://next"` opens a link.

## Stopping it

Choose **Revoke** under Allow automation to unpair scripts and the command line. A new code appears, so pair again if you need to. Turning **Allow automation** off stops links and scripts altogether.
