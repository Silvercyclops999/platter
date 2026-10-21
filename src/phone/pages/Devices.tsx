/**
 * Devices on the phone: keeping this phone in step with your other devices,
 * then music kept on the phone itself. Syncing music to a USB drive or a
 * music player needs a computer, and a phone can't see those drives, so the
 * phone doesn't bring them up at all.
 */
import { GoPill, PageFrame } from "./kit";
import { SyncRow } from "./You";
import "../sync-setup.css";

export function PhoneDevices() {
  return (
    <PageFrame name="devices" title="Devices">
      <SyncRow />
      <div className="ph-devices__keep">
        <p className="ph-you__sync-note">To play music with no signal, download albums, playlists and mixes to this phone.</p>
        <GoPill route={{ name: "settings", section: "Downloads and devices" }}>Downloads on this phone</GoPill>
      </div>
    </PageFrame>
  );
}
