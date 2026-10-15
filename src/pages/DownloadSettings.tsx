/** Settings parts for downloads: the storage meter, Remove all, location and the queue's state. */
import { useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { formatBytes } from "../lib/format";
import { chooseDownloadFolder, pauseText, removeAllDownloads, startDownloads, useDownloads } from "../stores/downloads";
import "../components/downloads.css";

/** Keeps the downloads status current while Settings is open (started once for the app). */
export function useDownloadStatus() {
  useEffect(() => void startDownloads(), []);
  return useDownloads((s) => s.status);
}

/** "Downloads: 1.2 GB". */
export function downloadsTitle(used: number | undefined): string {
  return used === undefined ? "Downloads" : `Downloads: ${formatBytes(used)}`;
}

/** Used of the limit, with free space, and a meter. */
export function StorageNote() {
  const st = useDownloadStatus();
  if (!st) return null;
  const share = st.limitBytes > 0 ? Math.min(1, st.usedBytes / st.limitBytes) : 0;
  const full = st.paused === "full" || st.paused === "noSpace";
  return (
    <>
      {formatBytes(st.usedBytes)} of {formatBytes(st.limitBytes)} used
      {st.freeBytes != null ? `, ${formatBytes(st.freeBytes)} free on this drive` : ""}.
      <span
        className={`dl-meter ${full ? "dl-meter--full" : ""}`}
        role="meter"
        aria-label="Download storage used"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(share * 100)}
      >
        <span className="dl-meter__fill" style={{ width: `${share * 100}%` }} />
      </span>
    </>
  );
}

/** What the queue is doing. */
export function QueueNote() {
  const st = useDownloadStatus();
  if (!st || st.queued === 0) return null;
  const waiting = pauseText(st.paused);
  const songs = st.queued === 1 ? "1 song" : `${st.queued} songs`;
  return <>{waiting ? `${songs} to download. ${waiting}` : `Downloading ${songs}.`}</>;
}

export function RemoveAllControl() {
  const st = useDownloadStatus();
  const [asking, setAsking] = useState(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const none = !st || (st.usedBytes === 0 && st.items.length === 0);
  return (
    <>
      <Button size="sm" disabled={none} onClick={() => setAsking(true)}>
        Remove all downloads
      </Button>
      <Dialog open={asking} onClose={() => setAsking(false)} label="Remove all downloads?" className="confirm" initialFocus={cancel}>
        <h2 className="confirm__title">Remove all downloads?</h2>
        <p className="confirm__body">You won't be able to play them offline. Liked Songs stops downloading too.</p>
        <div className="confirm__actions">
          <Button ref={cancel} variant="ghost" onClick={() => setAsking(false)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              setAsking(false);
              void removeAllDownloads();
            }}
          >
            Remove
          </Button>
        </div>
      </Dialog>
    </>
  );
}

/** The folder, shortened to one line. */
export function FolderNote() {
  const st = useDownloadStatus();
  if (!st) return null;
  return (
    <span className="dl-folder" title={st.folder}>
      {st.folder}
    </span>
  );
}

export function ChangeFolderControl() {
  return (
    <Button size="sm" onClick={() => void chooseDownloadFolder()}>
      Change location
    </Button>
  );
}
