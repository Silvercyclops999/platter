# Lidarr

How to connect Lidarr to Platter, what its safety check looks for, and how requests, upgrades and the staging folder work.

## Connecting Lidarr

1. Open Settings, Integrations, Lidarr.
2. In **Lidarr address at home**, type Lidarr's address on your home network with its port, like http://192.168.1.20:8686.
3. To reach Lidarr away from home, also fill in **Lidarr address away from home** (a reverse proxy, tunnel or VPN you set up). Use https so the key travels encrypted. If your proxy needs headers, add them in **Extra sign-in details for the away address**.
4. In Lidarr, open Settings, General and copy the API key. Paste it into **Lidarr key** and choose **Save key**. It's kept in this computer's keychain.
5. Choose **Check setup**.

## The safety check

The check reads Lidarr's folders and settings and compares them with your Plex music, by the files themselves as well as their paths. It only reads. Platter runs it again before every request.

- Must fix (red). Platter won't send Lidarr anything until these are fixed. Some only stop Protected artists; Managed artists then keep working.
- Worth changing (amber). Nothing stops, but the setup could be safer.
- OK. Fine as it is.

Common red findings:

- Lidarr has no root folder. Add a separate one in Lidarr.
- Lidarr and Plex name a folder differently. Give both the same path (in Docker, mount it at the same place).
- A protected artist is in your Plex music, or inside a Lidarr root folder. Change the artist's path in Lidarr. Platter never moves artists itself.
- Lidarr writes tags while protected artists are under its root folders. Set Write Audio Tags to Never.
- The staging folder is missing, isn't a Lidarr root folder, or Plex scans it.

Amber findings include no recycling bin, Rename Tracks on, and an away address that would send your key unencrypted. Some findings have a button so Platter can make the change. It shows the exact change first and logs it under **Changes**, with **Put it back** where it can.

## Managed and Protected artists

Each artist gets a mode, listed under **Managed and protected artists**.

- Managed (the default). Lidarr owns the artist's folder and imports, renames and upgrades there.
- Protected. The files are curated. Lidarr only downloads into the staging folder, and Platter adds albums to your library itself, using the names and tags the artist's other albums use.

An artist is Protected when its files are tagged as anything but released, when an album is a demo, unreleased, leak or bootleg, or when it's on Lidarr's import list exclusions. You can change the mode yourself. Switching to Managed asks twice, because files Lidarr changes stay changed.

Protected artists also have **Add new official albums automatically** and **Look for better copies**.

## Practice mode

For the first week after setup, Platter only writes down what it would add or swap. Read the changes log, then choose **Looks right, start for real**. **Practice for a week** turns it back on.

## The staging folder

**Lidarr staging folder** is a Lidarr root folder, as Lidarr sees it, that no Plex library scans. Protected downloads and better copies wait there. Complete official albums appear under **Waiting for you**; anything else says why it waits. **Free up staging space after adding** frees the copy once the album is in your library.

## Upgrades

With a staging folder set, an album's menu has **Look for a better copy**. When the copy arrives, the album page shows what would change. Swaps run on this computer, so fill in **Where this computer reaches your music** first. Replaced files go to a _to_delete folder Plex ignores, never deleted, and **Undo** puts them back. **Upgrades: ask before each swap** decides if Platter waits for you.

## Requests

With **Show missing official releases** on, artist pages list official albums and EPs you don't have, with **Request**. Your first request asks you to type the album's name. If Lidarr can't be reached, the request waits and sends later. **Requests** shows where each one stands.

## Auto requests

Turn on **Auto-request new albums**, then choose it from an artist page's menu. New official albums the release radar finds are requested by themselves, up to **Most automatic requests a day**. Bootlegs and promos never are.

If the check fails, see [Troubleshooting](troubleshooting.md#lidarr-check-failing).
