/** The help pages in reading order (docs/help/<slug>.md). The docs site's contents follow it too. */
export const HELP_ORDER = [
  "getting-started",
  "layouts",
  "connecting",
  "playing-and-queue",
  "search",
  "your-library",
  "lyrics-and-transcription",
  "downloads-and-offline",
  "listening-together",
  "stats-and-wrapped",
  "archive-and-release-status",
  "tag-editor",
  "library-import",
  "lidarr",
  "stream-deck",
  "automation",
  "privacy",
  "troubleshooting",
] as const;

export type HelpSlug = (typeof HELP_ORDER)[number];
