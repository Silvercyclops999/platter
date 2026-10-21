# Lidarr

How to connect Lidarr to Platter, what its safety check looks for, and how requests, upgrades and the staging folder work.

## Connecting Lidarr

1. Open Settings, Integrations, Lidarr.
2. In **Lidarr address at home**, type Lidarr's address on your home network with its port, like http://192.168.x.x:8686.
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

With a staging folder set, an album's menu has **Find a better copy**. When the copy arrives, the album page shows what would change. Swaps run on this computer, so fill in **Where this computer reaches your music** first. Replaced files go to a _to_delete folder Plex ignores, never deleted, and **Undo** puts them back. **Ask before each swap** decides if Platter waits for you.

## Better copies

With **Better copies** on, Platter finds better copies of your released albums by itself, a few a day, lowest quality first, up to **Aim for**. They download into the staging folder, and each song swaps in only when it's proven the same recording. An album with anything unreleased in it, even one song, is never touched. For the first week Platter only lists what it would swap in the changes log. After that at most 10 albums swap a day, and a swap that fails or that you undo stops swaps for a day. An album you undo stays as it is. Albums Lidarr got for you get better copies from Lidarr itself.

## Requests

With **Show missing official releases** on, artist pages list official releases you don't have in their discography, with **Get**. Which kinds show (albums, singles and EPs, live albums, compilations) follows **Album kinds for Get albums**. Your first request asks you to type the album's name. If Lidarr can't be reached, the request waits and sends later. **Requests** shows where each one stands.

## New albums from artists you follow

Follow an artist and their new albums and EPs come in by themselves, up to **Most new albums a day**, and you get a notification when each one is here. Only what comes out after you follow, never their older albums. Bootlegs, promos, live albums and anything sharing a title with an album you have, unreleased ones included, are never asked for. To follow someone without getting everything, pick **Just show new albums** in their menu (on the phone, tap **Following**). **Get singles too** adds singles.

## Bring in my listening

If you imported listening history, Bring in my listening asks Lidarr for the albums you played a lot but don't have, a few a day. See [Library import](library-import.md#bring-in-my-listening).

## Platter Bridge

Platter does its Lidarr work while it's open, and catches up when you open it again. Platter Bridge is an optional small container for your server that does the same work while every computer is off. Pair it in Settings, Integrations, Bridge. Setting it up is described in [Platter Bridge](https://github.com/Silvercyclops999/platter/blob/main/docs/BRIDGE.md).

If the check fails, see [Troubleshooting](troubleshooting.md#lidarr-check-failing).
