# Platter Bridge

Platter Bridge is a small, optional program for the server your music lives
on. It's the part of Platter that looks after Lidarr, without the window, so
that work carries on while every computer with Platter is closed or asleep.

You don't need it. Platter works fully without it: when you open Platter, it
catches up with everything Lidarr did while it was closed. The Bridge just
means that happens as it goes instead of later. Nothing is installed on your
server unless you set the Bridge up yourself.

A computer can be the home side too. When Platter is open on a computer at
home that has Lidarr set up, your phone and your other paired devices reach
Lidarr through it, at home and away, with the same checks the Bridge makes.
The Bridge is for when every computer is off.

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
- Lets a phone or a computer away from home reach Lidarr through it, if
  Lidarr has no away address and no computer at home is on.
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

## Install it with one command

On the server, with your own Lidarr address and key:

```sh
docker run -d --name platter-bridge --restart unless-stopped \
  --user 1000:1000 \
  -e LIDARR_URL=http://10.20.30.40:8686 \
  -e LIDARR_API_KEY=your-lidarr-api-key \
  -v /path/to/appdata:/data \
  ghcr.io/silvercyclops999/platter-bridge:latest
```

Set `--user` to the owner of the appdata folder (`id` shows yours). The
image runs as a normal user, never root, and publishes no ports. Swap
`:latest` for a version like `:0.2.29` to stay on one release.

Then pair it and let it into Plex:

1. `docker logs platter-bridge` shows a pair code that starts with `PBB-`.
   `docker exec platter-bridge platter-bridge status` shows it too.
2. In Platter on your computer or your phone: **Settings > Bridge > Add the
   Bridge**, and enter the code. It works once, for an hour; while the
   Bridge isn't paired it shows a new one every hour.
3. The log then asks you to open plex.tv/link and enter a short code, so the
   Bridge can read your Plex library. Settings > Bridge shows the same code.

Leave out `LIDARR_URL` and `LIDARR_API_KEY` and the Bridge takes them from
Platter on a computer at home instead, once one is open.

## Docker Compose

```yaml
services:
  platter-bridge:
    image: ghcr.io/silvercyclops999/platter-bridge:latest
    container_name: platter-bridge
    restart: unless-stopped
    user: "1000:1000"
    environment:
      LIDARR_URL: "http://10.20.30.40:8686"
      LIDARR_API_KEY: "your-lidarr-api-key"
      TZ: "Etc/UTC"
      # MUSIC_PLEX_PATH: "/data/media/Music"
      # STAGING_DIR: "/staging"
    volumes:
      - ./appdata:/data
      # - /path/to/music:/music:ro      # read-only: scans and checks only
      # - /path/to/music:/music         # read-write: adds albums too
      # - /path/to/staging:/staging
```

The full file, pinned to the current release, is in
`docker/bridge/docker-compose.yml`.

## Unraid

1. In Unraid, **Docker > Add Container**, and choose the template
   `platter-bridge`, or use the template URL from
   `docker/bridge/platter-bridge.xml` (it points at the release it came
   with, never at main).
2. Fill in **Lidarr address** and **Lidarr API key**, or leave them empty to
   take them from Platter on a computer.
3. Optional: set **Music folder** to your music share. Leave it read-only
   if you only want scans and safety checks; set it to read-write if you
   want the Bridge to add protected albums itself. Leave **Plex's path for
   the music folder** empty at first (see below). For additions, also set
   **Lidarr staging folder**.
4. Apply, open the container's log, and pair it as above.

The template runs the container as `99:100` (Unraid's usual owner for
appdata and shares).

## Updates

The image follows `:latest`, and a new one comes out with every Platter release. On
Unraid the Docker tab shows "update ready" for it, and the Auto Update Applications
plugin from Community Applications can apply updates on a schedule like your other
containers. With Docker Compose or `docker run`, Watchtower does the same, or run
`docker pull` and recreate the container yourself. The Bridge never updates itself:
that would mean giving it control of Docker on your server. Pairing, settings and
progress live in the appdata folder, so an update keeps them.

## The music folder

With a music folder mounted, the Bridge needs to know which of Plex's
folders it is. Once it has read your Plex library it compares the files
under `/music` with the paths Plex gives, and when `MUSIC_PLEX_PATH` isn't
set to the folder they match, it says what to set, in its log and in
Settings > Bridge on your computer, for example "Set MUSIC_PLEX_PATH to
/data/media/Music". Set it and restart the container.

## Pairing

Pairing happens once, one of two ways. The Bridge shows a code in its log
and you enter it in Platter (Settings > Bridge > Add the Bridge), on a
computer or a phone. Or Platter shows a code (Settings > Bridge > Pair a
Bridge) and the Bridge reads it from `BRIDGE_PAIR_CODE` the first time it
starts. Either way the two prove they both know the code without sending it
anywhere, and agree on a key only they have. Each code works once, for an
hour, and only for the first device to use it; a code of one kind never
pairs as the other. The Bridge's own code is only in its log and on its
status page, which only the server itself can open. Restarting or recreating
the container before pairing makes a new code and the old one stops working,
so use the latest one in the log.

After pairing with `BRIDGE_PAIR_CODE`, clear it from the container's
settings. The Bridge remembers the code was used (a fingerprint of it, not
the code), so leaving it set does no harm: at each start it says the code is
used up and can be removed, and skips it. An unused code that has expired
is skipped the same way.

Once paired, Platter sends the Bridge how to reach Lidarr (Lidarr's home
address and API key) and its Lidarr settings, every time the Bridge starts,
so a changed key reaches it too. Whatever is set on the Bridge itself
(`LIDARR_URL`, `LIDARR_API_KEY`, `PLEX_URL`, `PLEX_TOKEN`) wins, and
Settings > Bridge says which of those the Bridge has of its own.

To unpair, use **Unpair** in Settings > Bridge. The Bridge hears it and
forgets your computer. Then remove the container if you like.

## Plex

Platter never gives the Bridge your Plex account's sign-in. The Bridge
reaches Plex one of these ways, the first that applies:

- `PLEX_URL` and `PLEX_TOKEN` set on the Bridge.
- A server shared with you: Plex gives each such server a token of its own,
  for that server only, and Platter sends the Bridge that.
- Its own sign-in. The Bridge shows a short code; you open plex.tv/link and
  enter it. It then shows in Plex's Authorized Devices as its own device,
  where you can sign it out without touching any other.

A Bridge set up with an older Platter kept the account's sign-in. When it
updates, it gives that up once and asks for its own sign-in instead.

## Privacy

- The Bridge only makes outgoing connections: to the relay, Lidarr, Plex,
  plex.tv and MusicBrainz (to check albums Bring in my listening asks for).
  It opens no ports. Its health check and status page listen on 127.0.0.1
  inside the container, for Docker and `platter-bridge status` only.
- Everything between the Bridge and your computers goes through the same
  relay as Listen along (ntfy.sh by default, or your own ntfy server: set it
  in Settings > Listen along, and `BRIDGE_RELAY` on the Bridge). The relay
  sees random topic names and sealed messages. No address, key, token,
  artist or album name ever goes through it in the clear.
- Sealed messages count for 12 hours, can't be replayed, and can't be moved
  to another topic.
- What a paired device asks the Bridge to read from Lidarr is limited to
  the pages Platter reads, each query name given once, folders only inside
  Lidarr's own; every change is rebuilt from only the fields Platter sends
  and checked against Lidarr first. Keys and passwords are taken out of
  every answer.
- On the server, the Bridge keeps its secrets in `keys.json` in the data
  folder, readable only by its own user (put right if that ever changes),
  and its state in `bridge.db`, which holds no keys or tokens.

ntfy.sh is free and limits how many messages one address sends a day. The
Bridge stays well under that (a heartbeat every half hour, and messages
when something happens). If you run your own ntfy server, there's no limit.

## Settings

| Variable | What it's for |
| --- | --- |
| `LIDARR_URL`, `LIDARR_API_KEY` | Lidarr as the server reaches it. They win over what a computer sends. |
| `PLEX_URL` | Plex's address as the server reaches it. Optional. |
| `PLEX_TOKEN` | A token for that Plex server, with `PLEX_URL`. Optional. |
| `BRIDGE_PAIR_CODE` | A code from Settings > Bridge > Pair a Bridge. Used once; clear it after pairing. Optional. |
| `BRIDGE_NAME` | What Platter calls this Bridge. Default "Platter Bridge". |
| `MUSIC_PLEX_PATH` | The path Plex uses for the folder mounted at `/music`. |
| `MUSIC_DIR` | Where the music folder is mounted, if not `/music`. |
| `STAGING_DIR` | Where Lidarr's staging folder is mounted (`/staging`). |
| `BRIDGE_RELAY` | Your own ntfy server, if you use one. |
| `BRIDGE_LOG` | Log detail, like `debug`. Default `info`. |
| `BRIDGE_DATA` | The data folder. Default `/data`. |

## Logs and health

`docker logs platter-bridge` says plainly what it's doing: its pair code
while it isn't paired, the plex.tv/link code while it waits for Plex,
catching up, what it added. `docker exec platter-bridge platter-bridge
status` prints the same in short. Docker's health check runs
`platter-bridge health`, which is healthy while the Bridge keeps working.

## For the maintainer

The image is pushed to GitHub Container Registry by the bridge workflow on
each version tag, as `platter-bridge:<version>` and `:latest`. The package
must be public in the package's settings on GitHub, or `docker pull` asks
for a sign-in. With every version bump, run `node scripts/pin-bridge-template.mjs`
before tagging, so the Unraid template and compose file name that release.
CI fails while they don't match `package.json`, and the tag's publish job
checks the template names the tag.

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

## A computer at home

You may not need the Bridge at all. Pair your phone with your computer
(Settings > Sync, Keep my devices in sync) and, as long as Platter is open
on that computer and Lidarr is set up there, the phone can get albums, see
what's missing and send requests through it. On the same network it goes
straight across; away from home it goes through the same sealed relay.

The computer answers only your own paired devices signed in to the same
Plex account, and only after they've said who they are. It checks every
request the way the Bridge does, takes keys and passwords out of what it
sends back, and keeps the Lidarr key to itself. You can turn it off on the
computer with Let my other devices use Lidarr through this computer, in
Settings > Lidarr under Home side.

Platter tries Lidarr's home address first, then its away address, then a
computer at home, then the Bridge. Settings > Lidarr > Home side lists
which of them you have, whether each is on now and which one answers. When
none is on and there's no away address, it says "Lidarr isn't reachable
away from home. Turn on a computer at home or add the Bridge."

## What needs a computer

Requests by hand (asking Lidarr for an album) start on a computer. The library's
file tags, which decide which artists are protected, are read on a computer.
Better copies ("Look for better copies" and upgrades you started) are kept
per computer, so they're swapped on the computer that asked for them.
