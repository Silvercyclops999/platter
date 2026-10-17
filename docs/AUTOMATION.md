# Automation

Platter can be controlled from outside the app: from `platter://` links,
from a small command line, and from any program on the same computer that
talks to its local control API. This is how you wire it into Home Assistant,
Raycast, keyboard launchers or your own scripts.

All of it is off until you turn on Allow automation in Settings. It stays on
the computer you turn it on for. Nothing listens on your network: the control
API answers only programs on this computer, and web pages can't use it.

## Links

With automation on, opening one of these links controls the running app.
You can put them in a shortcut, a launcher or a script (`start` on Windows,
`open` on macOS).

    platter://play
    platter://pause
    platter://toggle
    platter://next
    platter://previous
    platter://play?playlist=Road%20Trip
    platter://play?playlist=Road%20Trip&shuffle=1
    platter://search?q=night%20drive
    platter://open/liked

A playlist can be named by its id or its name; a name matches exactly first,
then the playlist whose name starts with it. `open` knows home, search,
library, liked, settings, stats, recents, bookmarks, lyrics, release-radar and
whats-new. Playback links leave the window where it is; search and open bring
it forward.

If automation is off, the app says so and does nothing. Links don't need
pairing, but a web page can open them too, so they only control playback.
Liking a song is on the command line (`--like`), which needs pairing.

## Command line

The app's own program doubles as a command line. It talks to the copy that's
already running and exits; it never starts the app.

    platter --pair 123456
    platter --play
    platter --pause
    platter --toggle
    platter --next
    platter --previous
    platter --like
    platter --playlist "Road Trip" --shuffle
    platter --now-playing
    platter --now-playing --json

Pair once with the six-digit code shown under Allow automation. The command
line keeps its token in the system keychain. Revoke in Settings unpairs it.

On Windows the program is `Platter.exe` in the install folder; on macOS it's
inside the app at `Platter.app/Contents/MacOS/Platter`. Add that folder to
your PATH or use the full path.

`--now-playing` prints the song and artist. With `--json` it prints the
playing state: title, artist, album, track id, whether it's playing and
liked, volume and whether a private session is on (which leaves the song
out).

The exit code is 0 when it worked, 1 when the app said no (the message says
why), 2 for a mistake in the arguments, and 3 when Platter isn't running or
automation is off.

## The control API

Anything else can use the same HTTP API the command line uses, on
`127.0.0.1:32541`. It's described under Stream Deck in [IPC.md](IPC.md).
Pair by sending your code once:

    curl -s -X POST http://127.0.0.1:32541/v1/pair \
      -H "Content-Type: application/json" \
      -d '{"code":"123456","client":"automation"}'

That answers with a token. Send it with each request:

    curl -s -X POST http://127.0.0.1:32541/v1/command \
      -H "Authorization: Bearer <token>" \
      -H "Content-Type: application/json" \
      -d '{"command":"next"}'

    curl -s "http://127.0.0.1:32541/v1/state?cover=none" \
      -H "Authorization: Bearer <token>"

Commands are `play`, `pause`, `playPause`, `next`, `previous`, `like`,
`volumeSet` and `volumeStep` (with `value` in percent), `playPlaylist` (with
`playlist` and optional `shuffle`), `search` (with `query`) and `open` (with
`page`). Pairing a script replaces any script paired before; the Stream Deck
has a pairing of its own and isn't affected.

Scripts always pair with the code. Asking in Platter's window instead
(`/v1/pair/request`) is only for the Stream Deck plugin, which Settings
installs with Install for Stream Deck.

## Home Assistant

Home Assistant usually runs on another machine, and Platter's API only
answers this computer, so Home Assistant needs a helper on the computer
running Platter. A companion app that can run commands on your PC, such as
HASS.Agent on Windows, works well: add custom commands that run
`platter --toggle`, `platter --next` and so on, and they show up in Home
Assistant as buttons you can put on a dashboard or call from automations.
To show what's playing, have the helper run `platter --now-playing --json`
as a sensor.

## Raycast

Raycast script commands are small shell scripts with a header. One per
action is enough:

    #!/bin/bash
    # @raycast.schemaVersion 1
    # @raycast.title Next song
    # @raycast.mode silent
    open "platter://next"

Use `platter --now-playing` with `@raycast.mode inline` to show the playing
song in the Raycast bar.

## Scripts

From a shell, the command line is the simplest. A script that pauses music
when a call starts might run `platter --pause`, and resume with
`platter --play`. In PowerShell, `Start-Process "platter://next"` opens a link,
and `Invoke-RestMethod` works with the API the same way as curl above.
