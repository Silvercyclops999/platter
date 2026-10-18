# Platter Bridge

Platter Bridge is a small, optional program for the server your music lives
on. It's the part of Platter that looks after Lidarr, without the window, so
that work carries on while every computer with Platter is closed or asleep.

You don't need it. Platter works fully without it: when you open Platter, it
catches up with everything Lidarr did while it was closed. The Bridge just
means that happens as it goes instead of later. Nothing is installed on your
server unless you set the Bridge up yourself.

## What it does

Every few minutes, the same way an open Platter would:

- Reads what Lidarr did since the last thing anyone handled.
- When a Managed artist's album is imported and Plex hasn't picked it up
  after a couple of minutes, asks Plex to scan just that folder.
- Keeps protected albums safe: albums Lidarr no longer needs are
  unmonitored, and if someone turns on Rename Tracks, Write Audio Tags or
  clears the recycling bin in Lidarr, protected work pauses until you look.
- With your music folder mounted read-write, adds new protected albums from
  the staging folder to your library (only the ones you said can be added
  automatically), following each artist's own folder and tag habits.
- Lets a computer away from home reach Lidarr through it, if Lidarr has no
  away address.
- Runs Bring in my listening, so the waves of requests keep going while
  your computers are off (see below).

What it never does:

- It never changes Lidarr's own settings, profiles, root folders or
  exclusions. If something would need that, it waits in Settings > Bridge
  on your computer, with exactly what it would send, for you to say yes or
  dismiss it.
- It never works out on its own which artists are protected. That comes
  from the tags in your files, which Platter on your computer reads. The
  Bridge waits until a computer has shared it.
- Practice mode holds on the Bridge too: while it's on, the Bridge writes
  down what it would add or swap, and does nothing.

The Bridge and your computers never both act on the same Lidarr event. Each
one claims an event before acting, and only the earliest claim acts.

## Unraid

1. In Platter on your computer: **Settings > Bridge > Pair a Bridge**. Keep
   Platter open; the code works once, for an hour.
2. In Unraid, **Docker > Add Container**, and choose the template
   `platter-bridge` (or use the template URL:
   `https://raw.githubusercontent.com/Silvercyclops999/platter/main/docker/bridge/platter-bridge.xml`).
3. Paste the code into **Pair code**.
4. Optional: set **Music folder** to your music share. Leave it read-only
   if you only want scans and safety checks; set it to read-write if you
   want the Bridge to add protected albums itself. Then set **Plex's path
   for the music folder** to the same folder as Plex sees it (for example
   `/data/media/Music`), and, for additions, **Lidarr staging folder**.
5. Apply. Within a few seconds Settings > Bridge on your computer shows the
   Bridge, and the Lidarr page says the Bridge does the work while your
   computers are off.

The template runs the container as `99:100` (Unraid's usual owner for
appdata and shares).

## Docker Compose

```yaml
services:
  platter-bridge:
    image: ghcr.io/silvercyclops999/platter-bridge:latest
    container_name: platter-bridge
    restart: unless-stopped
    user: "1000:1000"
    environment:
      BRIDGE_PAIR_CODE: "PB1-paste-the-code-from-Platter-here"
      TZ: "Etc/UTC"
      # MUSIC_PLEX_PATH: "/data/media/Music"
      # STAGING_DIR: "/staging"
    volumes:
      - ./appdata:/data
      # - /path/to/music:/music:ro      # read-only: scans and checks only
      # - /path/to/music:/music         # read-write: adds albums too
      # - /path/to/staging:/staging
```

The full file is in `docker/bridge/docker-compose.yml`. Set `user` to the
owner of the appdata folder (`id` shows yours). The image runs as a normal
user, never root, and has no ports.

## Pairing

Pairing happens once. Platter shows a code; the Bridge reads it from
`BRIDGE_PAIR_CODE` the first time it starts. The two prove they both know
the code without sending it anywhere, and agree on a key only they have.
After that the code is used up: it can stay in your settings or be removed,
and nobody else can pair with it.

Once paired, Platter sends the Bridge how to reach Lidarr and Plex (Lidarr's
home address and API key, your Plex sign-in). It does this every time the
Bridge starts, so a changed key reaches it too.

To unpair, use **Unpair** in Settings > Bridge. The Bridge hears it and
forgets your computer. Then remove the container if you like.

## Privacy

- The Bridge only makes outgoing connections: to the relay, Lidarr, Plex,
  plex.tv and MusicBrainz (to check albums Bring in my listening asks for).
  It opens no ports. Its health check listens on 127.0.0.1
  inside the container, for Docker only.
- Everything between the Bridge and your computers goes through the same
  relay as Listen along (ntfy.sh by default, or your own ntfy server: set it
  in Settings > Listen along, and `BRIDGE_RELAY` on the Bridge). The relay
  sees random topic names and sealed messages. No address, key, token,
  artist or album name ever goes through it in the clear.
- Sealed messages count for 12 hours, can't be replayed, and can't be moved
  to another topic.
- On the server, the Bridge keeps its secrets in `keys.json` in the data
  folder, readable only by its own user, and its state in `bridge.db`.

ntfy.sh is free and limits how many messages one address sends a day. The
Bridge stays well under that (a heartbeat every half hour, and messages
when something happens). If you run your own ntfy server, there's no limit.

## Settings

| Variable | What it's for |
| --- | --- |
| `BRIDGE_PAIR_CODE` | The code from Settings > Bridge. Used once. |
| `BRIDGE_NAME` | What Platter calls this Bridge. Default "Platter Bridge". |
| `MUSIC_PLEX_PATH` | The path Plex uses for the folder mounted at `/music`. |
| `MUSIC_DIR` | Where the music folder is mounted, if not `/music`. |
| `STAGING_DIR` | Where Lidarr's staging folder is mounted (`/staging`). |
| `BRIDGE_RELAY` | Your own ntfy server, if you use one. |
| `LIDARR_URL`, `LIDARR_API_KEY` | Set these only to override what your computer sends. |
| `BRIDGE_LOG` | Log detail, like `debug`. Default `info`. |
| `BRIDGE_DATA` | The data folder. Default `/data`. |

## Logs and health

`docker logs platter-bridge` says plainly what it's doing: waiting to pair,
waiting for your computer's settings, catching up, what it added. Docker's
health check runs `platter-bridge health`, which is healthy while the Bridge
keeps working.

## Bring in my listening

When Bring in my listening is on and a Bridge is paired, your computer hands
the waves to the Bridge and stops running them itself, so only one of them
ever asks Lidarr for anything. The Bridge gets the list of albums still to
ask for (artist, album, the MusicBrainz release group when your computer
already knows it, and how many plays), the waves settings, and which albums
were already asked for. Nothing else from your listening history leaves your
computer: no songs, dates or play times.

The Bridge asks for each album the same way your computer would. It checks
the album on MusicBrainz first, runs Lidarr's setup check, follows the
Managed and Protected rules and the staging folder, keeps to the daily
pace, and pauses when Lidarr's queue is full, space runs low, Lidarr can't
be reached or requests keep failing. Practice mode holds here too. Each
step is in `docker logs`, and requests and pauses go to the Changes log.

The card on Played a lot says "Running on your Bridge" while the Bridge has
the waves, and shows its progress and anything that needs you. Turning Bring
in my listening off on your computer stops the Bridge. If the Bridge stops
answering for over an hour, your computer takes the waves back and carries
on from where the Bridge got to.

## What needs a computer

Requests by hand (asking Lidarr for an album) start on a computer. The library's
file tags, which decide which artists are protected, are read on a computer.
Better copies ("Look for better copies" and upgrades you started) are kept
per computer, so they're swapped on the computer that asked for them.
