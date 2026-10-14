# Accessibility

I want Platter to work well for everyone, with a keyboard, a screen reader, or
with less motion on screen.

## What's in place

- Everything can be reached and used from the keyboard, with a visible focus
  ring. Ctrl+K (Cmd+K on Mac) opens a command palette for jumping anywhere.
- Buttons, sliders, menus and dialogs have labels for screen readers, and
  dialogs keep focus inside them and hand it back when they close. Buttons
  say what they act on ("Play Halo by Beyonce"), toggles say whether they're
  on, and a control that can't be used says why.
- The window has landmarks a screen reader can jump between: the top bar
  with search, the library, the main area (named after the page you're on)
  and the player. Each page has one main heading.
- Song lists are grids, so a screen reader can say "row 12 of 61" and
  whether a row is selected. Library items say where they are in the list.
- When the song changes the player says "Now playing" with the title and
  artist, and messages like "Added to queue" are read out politely.
- The seek bar reads the time as "0:05 of 3:44" and the volume as a
  percentage.
- Every build runs an automated accessibility check (axe) over the main
  screens.
- Reduce motion follows your system setting, or can be set in Settings. It
  turns off sliding and scaling but keeps gentle fades.
- Text and controls are checked for contrast against their backgrounds,
  including headers tinted from album art.
- The window works down to 800 pixels wide without anything being cut off.

## Supported

Windows and macOS, with the screen reader built into each (Narrator,
VoiceOver) or NVDA on Windows.

## Known gaps

- There hasn't been a full pass with each screen reader yet, so expect rough
  spots. Reports help a lot.
- There's no high contrast theme or text size setting of its own yet. The
  system zoom and display scaling settings work.
- Synced lyrics and some animations haven't had a full screen reader pass.

## Reporting a problem

If something gets in your way, please open an issue and say what you were
trying to do, which assistive technology you use, and your operating system.
Accessibility bugs are treated as bugs, not feature requests.
