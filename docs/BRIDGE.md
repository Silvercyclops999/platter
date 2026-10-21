# Platter Bridge

Platter Bridge is a small, optional program for the server your music lives
on. It's the part of Platter that works without the window, so the heavy
and the always-on jobs happen on the server while every computer with
Platter is closed or asleep. Today that's two things, each with its own
switch in Settings > Bridge:

1. Lyrics for every song. At night it transcribes the songs that have no
   lyrics anywhere, on the server, and every device gets them. Your
   computer never does that heavy work.
2. Lidarr from anywhere, even with every computer off.

You don't need it. Platter works fully without it: when you open Platter, it
catches up with everything Lidarr did while it was closed, and it can
transcribe songs itself. Nothing is installed on your server unless you set
the Bridge up yourself.

A computer can be the home side too. When Platter is open on a computer at
home that has Lidarr set up, your phone and your other paired devices reach
Lidarr through it, at home and away, with the same checks the Bridge makes.
The Bridge is for when every computer is off.

## Lyrics for every song

With your music folder mounted at `/music` (read-only is enough), the
Bridge works through the songs that have no lyrics: not from Plex, not
from LRCLIB (when Lyrics from LRCLIB is on in Platter), not written by you
and not transcribed on one of your computers. Most played songs go first.
It uses the same speech model as Platter on a computer, at night only (1 to
7 AM in your computer's time zone unless you pick other hours), with two
processor threads at the lowest priority. It waits while other programs
keep the server busy and rests a few seconds between songs.

Each song's lyrics go to every paired device through the sealed relay, and
on from there to your other devices. They show as transcribed lyrics, like
ones your computer made. Lyrics you wrote yourself always win.

While the Bridge does this, your computers don't run their own background
batch. A song you play that the Bridge hasn't reached yet is still
transcribed on the spot when transcription is on there. If the Bridge stops
answering, or can't do the work, your computers carry on as before.

The speech model (Small, about 190 MB) downloads into the appdata folder
the first time, from the same place and with the same checksum check as on
a computer. A file that can't be read, or is bigger than 300 MB, is skipped
and tried again a week later. The Bridge never writes to the music folder
for this.

A song is read whole and decoded by the Bridge itself; only the speech model
runs in a separate helper process. With the Small model the Bridge can use
around 1.5 GB of memory while it works, so if you like to cap containers,
add `--memory 2g` (or the same under Unraid's Extra Parameters).

The Bridge needs to know which of Plex's folders `/music` is. Once it has
read your Plex library it works that out by itself; setting
`MUSIC_PLEX_PATH` is only needed for adding albums.

## Lidarr

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

On the server, with your own folders:

```sh
docker run -d --name platter-bridge --restart unless-stopped \
  --user 1000:1000 \
  -v /path/to/appdata:/data \
  -v /path/to/music:/music:ro \
  ghcr.io/silvercyclops999/platter-bridge:latest
```

Set `--user` to the owner of the appdata folder (`id` shows yours). The
image runs as a normal user, never root, and publishes no ports. Swap
`:latest` for a version like `:0.2.29` to stay on one release. The music
folder is for lyrics; read-only is enough. Add
`-e LIDARR_URL=http://10.20.30.40:8686 -e LIDARR_API_KEY=your-lidarr-api-key`
if you'd like it to reach Lidarr before a computer has told it how.

Then sign it in to Plex. `docker logs platter-bridge` asks you to open
plex.tv/link and enter a short code (`docker exec platter-bridge
platter-bridge status` shows it too). Sign in with the Plex account that owns
your server. That's all: every Platter signed in to that account, computer
or phone, finds the Bridge by itself within a minute. There's no code to type
into Platter.

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
3. Set **Music folder** to your music share, for lyrics for every song.
   Leave it read-only for lyrics, scans and safety checks; set it to
   read-write if you want the Bridge to add protected albums itself. Leave **Plex's path for
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
/data/media/Music". Lyrics for every song uses that match straight away,
since it only reads. To let the Bridge add albums, set it and restart the
container.

## Linking

The Bridge and your devices find each other through your Plex account.
plex.tv lists every device signed in to an account, each with its own
sign-in, and shows that list only to the account itself. Each Platter reads
the list, and both ends of a link work out the same key from the two
devices' sign-ins, so nothing secret is sent anywhere to set it up. A Plex
Home user, a friend you share your server with, or anyone else isn't on
your list and can't link. Sign a device out in Plex's Authorized Devices and
its link goes too.

Only the account that owns your Plex server links. A Bridge signed in to
any other account links to nothing and says so in its log. On a device,
Lidarr works only while it's signed in as the server's owner: if someone
else signs in on it, its Bridge link goes, it stops answering for your
other devices, and it doesn't use Lidarr even with your key saved there.

If something breaks a link (the Bridge loses its data folder, say), the two
link again by themselves the next time they look, which is every 10
minutes and right away when a device signs in.

A Bridge without a Plex sign-in of its own (one that only has `PLEX_TOKEN`)
still asks for plex.tv/link, since that's how it finds your devices. Older
Platter builds can still pair with the code the Bridge shows in its log while
it isn't signed in, or with `BRIDGE_PAIR_CODE`; clear that from the
container's settings once it's paired.

Once linked, a computer with Lidarr set up sends the Bridge how to reach Lidarr (Lidarr's home
address and API key) and its Lidarr settings, every time the Bridge starts,
so a changed key reaches it too. Whatever is set on the Bridge itself
(`LIDARR_URL`, `LIDARR_API_KEY`, `PLEX_URL`, `PLEX_TOKEN`) wins, and
Settings > Bridge says which of those the Bridge has of its own.

To unpair, use **Unpair** in Settings > Bridge. The Bridge hears it and
forgets that device, and the two don't link again by themselves until one
of them signs in to Plex again. Then remove the container if you like.

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
  plex.tv, MusicBrainz (to check albums Bring in my listening asks for),
  Hugging Face once for the speech model, and LRCLIB only while Lyrics from
  LRCLIB is on in Platter. Your songs' audio never leaves the server.
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
| `BRIDGE_PAIR_CODE` | A pair code from an older Platter build. Not needed: the Bridge links by your Plex account. Used once; clear it after pairing. Optional. |
| `BRIDGE_NAME` | What Platter calls this Bridge. Default "Platter Bridge". |
| `MUSIC_PLEX_PATH` | The path Plex uses for the folder mounted at `/music`. |
| `MUSIC_DIR` | Where the music folder is mounted, if not `/music`. |
| `STAGING_DIR` | Where Lidarr's staging folder is mounted (`/staging`). |
| `BRIDGE_RELAY` | Your own ntfy server, if you use one. |
| `BRIDGE_LOG` | Log detail, like `debug`. Default `info`. |
| `BRIDGE_DATA` | The data folder. Default `/data`. |

## Logs and health

`docker logs platter-bridge` says plainly what it's doing: the plex.tv/link code while it waits for Plex,
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

You may not need the Bridge at all. Your phone and your computer find each
other through your Plex account (Settings > Sync, Keep my devices in sync)
and, as long as Platter is open
on that computer and Lidarr is set up there, the phone can get albums, see
what's missing and send requests through it. On the same network it goes
straight across; away from home it goes through the same sealed relay.

The computer answers only your own devices signed in to the same
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
