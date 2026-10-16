import type { Settings } from "../lib/types";

/**
 * The look-ups, sharing and server writes that start off on a new install
 * (settings.rs `risky_features_start_off`). Tests of those features turn
 * them on first, as the user would in Settings or the first-run guide.
 */
export const ONLINE_EXTRAS: Partial<Settings> = {
  discordStatus: true,
  shareActivity: true,
  showToMyDevices: true,
  saveQueueOnServer: true,
  catalogueSearch: true,
  fansAlsoLikeCatalogue: true,
  homeNewToYou: true,
  releaseRadar: true,
  checkUnmatchedMusicbrainz: true,
  infoWikipedia: true,
  infoListenbrainz: true,
  infoMusicbrainz: true,
  infoListenbrainzStats: true,
  sharePublicLinks: true,
};
