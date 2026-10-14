# Accessibility

I want Platter to work well for everyone, with a keyboard, a screen reader, or
with less motion on screen.

## What's in place

- Everything can be reached and used from the keyboard, with a visible focus
  ring. Ctrl+K (Cmd+K on Mac) opens a command palette for jumping anywhere.
- Buttons, sliders, menus and dialogs have labels for screen readers, and
  dialogs keep focus inside them and hand it back when they close.
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
