/**
 * Devices on the phone: first, keeping this phone in step with your other
 * devices (or copying from your computer). Syncing music to a USB drive, an
 * SD card or a music player needs a computer; a phone can't see those
 * drives, so the page says so and points at downloads, which keep music on
 * the phone itself.
 */
import { HardDrive } from "lucide-react";
import { GoPill, PageFrame } from "./kit";
import { SyncRow } from "./You";
import "../sync-setup.css";

export function PhoneDevices() {
  return (
    <PageFrame name="devices" title="Devices">
      <SyncRow />
      <div className="ph-empty" role="status">
        <span className="ph-x__link-icon" aria-hidden="true">
          <HardDrive size={20} />
        </span>
        <p className="ph-empty__title">On your computer</p>
        <p className="ph-empty__text">
          Syncing copies music to a USB drive, an SD card or a music player plugged into a computer. A phone can't see those drives, so devices are set up and synced in Platter on your computer.
        </p>
        <p className="ph-empty__text">To keep music on this phone, download albums, playlists and mixes instead.</p>
        <GoPill route={{ name: "settings", section: "Downloads and devices" }} primary>
          Downloads on this phone
        </GoPill>
      </div>
    </PageFrame>
  );
}
