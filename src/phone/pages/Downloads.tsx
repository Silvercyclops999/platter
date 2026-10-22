/**
 * The phone has no downloads page: what's downloaded is in Library (its
 * Downloaded pill), and storage, Remove all and Go offline are in Settings.
 * The old route opens that Settings section.
 */
import { useEffect } from "react";
import { useRouter } from "../../stores/router";

export function PhoneDownloads() {
  useEffect(() => useRouter.getState().replace({ name: "settings", section: "Downloads and devices" }), []);
  return null;
}
