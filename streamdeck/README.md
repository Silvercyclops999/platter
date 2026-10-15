# Platter for Stream Deck

A Stream Deck plugin that controls Platter on the same computer. It has keys for Play / Pause (showing the cover of what's playing), Next, Previous, Like, Request album and Listen along, and a Volume dial for Stream Deck +.

It talks only to Platter's control API on 127.0.0.1, which is off until you turn on Allow Stream Deck in Platter's Settings. Nothing goes over your network.

## Pairing

1. In Platter, open Settings, find Stream Deck and turn on Allow Stream Deck. A six-digit code shows under it.
2. In the Stream Deck app, drag any Platter action onto a key, and type the code in its settings panel.

The plugin keeps the token Platter gives it. Revoke in Platter's Settings, or Unpair in the plugin, forgets it.

Request album opens Platter's usual Lidarr request for the playing song's album, so it only works once Lidarr is set up in Platter. Listen along starts a session in Platter and shows its invite.

## Building

Needs Node.js 20 or newer.

```
npm install
npm run build      # bundles src/ into app.platter.streamdeck.sdPlugin/bin/plugin.js
npm test           # client and key tests
npm run typecheck
npm run pack       # writes dist/app.platter.streamdeck.streamDeckPlugin
```

Open the `.streamDeckPlugin` file to install it. While working on it, `npx streamdeck link app.platter.streamdeck.sdPlugin` installs the folder itself.

The key icons are drawn by `scripts/icons.mjs`; run `node scripts/icons.mjs` after changing one.
