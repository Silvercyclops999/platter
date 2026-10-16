# Stream Deck

How to add Platter's keys to an Elgato Stream Deck, allow them in Platter, and what each key does.

## What you need

- An Elgato Stream Deck and the Stream Deck app (version 6.6 or newer) on the same computer as Platter.
- Windows 10 or newer, or macOS 13 or newer.

The plugin talks only to Platter on this computer. Nothing goes over your network, and it only works while Allow Stream Deck is on.

## Installing the plugin

The plugin comes with Platter.

1. Open Settings and find the Stream Deck section.
2. Next to **Platter plugin**, choose **Install for Stream Deck**. This turns on **Allow Stream Deck** and hands the plugin to the Stream Deck app, which installs it.
3. When it's done, the row says Installed.

If the Stream Deck app isn't installed, the row links to Elgato's download page instead. When a newer Platter brings a newer plugin, the same button says **Update**. **Reinstall** is there if something went wrong.

Each Platter release also has a file called `app.platter.streamdeck.streamDeckPlugin`. Opening it installs the plugin too.

If your copy of Platter says it doesn't include the plugin, it can be built from the `streamdeck` folder of Platter's source. Most people won't need to do this.

## Allowing it in Platter

1. In the Stream Deck app, drag any Platter key onto your Stream Deck.
2. Press it. Platter's window comes up and asks "Allow Stream Deck to control Platter?". Until you answer, the key says Approve in Platter.
3. Choose **Allow**. Every Platter key now works, with nothing else to set up.

If you choose **Don't allow**, the plugin stops asking until you press one of its keys again. Platter never allows a Stream Deck by itself.

You can also pair with a code. With Allow Stream Deck on, Settings shows a six-digit pairing code. In the Stream Deck app, select any Platter key and type the code into its settings panel. That panel also has **Ask Platter** (to bring up the prompt again) and **Unpair**.

To stop a Stream Deck from controlling Platter, choose **Revoke** under Allow Stream Deck. Pair again with the new code if you change your mind.

## The keys

- Play / Pause plays or pauses, and shows the cover of what's playing.
- Next skips to the next song.
- Previous goes back to the start of the song, or to the one before.
- Like likes the playing song, or unlikes it. The heart fills in when it's liked.
- Volume is for the dial on a Stream Deck +. Turn it to change Platter's volume, press or touch it to play or pause.
- Request album opens Platter's usual Lidarr request for the playing song's album. It only works once Lidarr is set up (see [Lidarr](lidarr.md)).
- Listen along starts a Listen along session in Platter and shows its invite (see [Listening together](listening-together.md)).

## When a key doesn't work

Keys show a short word when something is in the way.

- Approve in Platter means Platter is waiting for you to choose Allow in its window.
- Pair means the Stream Deck isn't paired, or was revoked or not allowed. Press the key and choose Allow, or type the new code.
- Off means Platter isn't running, or Allow Stream Deck is off in its Settings.

If Settings says the port is in use by another program, something else on this computer is using the address Platter listens on, so the Stream Deck can't reach it. Close that program and try again.

For other problems, see [Troubleshooting](troubleshooting.md).
