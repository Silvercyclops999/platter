# Platter for Stream Deck

A Stream Deck plugin that controls Platter on the same computer. It has keys for Play / Pause (showing the cover of what's playing), Next, Previous, Like, Request album and Listen along, and a Volume dial for Stream Deck +.

It talks only to Platter's control API on 127.0.0.1, which is off until you turn on Allow Stream Deck in Platter's Settings. Nothing goes over your network.

## Installing

The plugin comes with Platter on Windows and macOS. In Platter, open Settings, find Stream Deck and click Install for Stream Deck. That turns on Allow Stream Deck and hands the plugin to the Stream Deck app, which installs it. When a newer Platter brings a newer plugin, the same button says Update.

Without the Stream Deck app, Settings links to Elgato's download page instead. Each release also has `app.platter.streamdeck.streamDeckPlugin` as a download; opening it installs the plugin too.

## Pairing

Drag any Platter action onto a key. The first time the plugin reaches Platter it asks to pair: Platter's window comes up with "Allow Stream Deck to control Platter?" and the keys say "Approve in Platter". Choose Allow and every key works; nothing else to set up.

If you choose Don't allow (or leave it), the plugin stops asking until you press one of its keys. Platter shows one prompt at a time and never allows by itself.

The six-digit code still works: in Platter's Settings, under Allow Stream Deck, there's a code to type into any Platter action's settings panel in the Stream Deck app. That panel also has Ask Platter, and Unpair.

The plugin keeps the token Platter gives it, and all keys share it. Revoke in Platter's Settings, or Unpair in the plugin, forgets it.

Request album opens Platter's usual Lidarr request for the playing song's album, so it only works once Lidarr is set up in Platter. Listen along starts a session in Platter and shows its invite.

## Building

Needs Node.js 20 or newer.

```
npm ci
npm run build      # bundles src/ into app.platter.streamdeck.sdPlugin/bin/plugin.js
npm test           # client, pairing and key tests
npm run typecheck
npm run pack       # writes dist/app.platter.streamdeck.streamDeckPlugin
```

Open the `.streamDeckPlugin` file to install it. While working on it, `npx streamdeck link app.platter.streamdeck.sdPlugin` installs the folder itself.

The release build packs the plugin and bundles it into Platter with `--config src-tauri/tauri.streamdeck.conf.json`. To try Install for Stream Deck from a local build, run `npm run pack` here first and build Platter with that same `--config`. Bump `Version` in the manifest (and `version` in package.json) when the plugin changes, so Settings offers the update.

The key icons are drawn by `scripts/icons.mjs`; run `node scripts/icons.mjs` after changing one.
