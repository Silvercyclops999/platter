/** "Your stats" from a song, artist or album menu: its stats strip in a dialog. */
import { nav } from "../stores/router";
import { useStatsDialog, type StatsSubject } from "../stores/stats";
import { useSettings } from "../stores/settings";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { StatsStripView, useItemStats } from "./StatsStrip";
import "./stats-strip.css";

const KIND_WORD = { song: "song", artist: "artist", album: "album" } as const;

function StatsBody({ subject, onClose }: { subject: StatsSubject; onClose: () => void }) {
  const stats = useItemStats(subject.kind, subject.id);
  const statsPage = useSettings((s) => s.settings?.statsPage !== false);
  return (
    <div className="stats-dialog" data-testid="stats-dialog">
      <h2 className="stats-dialog__title">Your stats</h2>
      <p className="stats-dialog__sub truncate">{subject.title}</p>
      {stats === null ? (
        <p className="stats-dialog__empty" role="status">
          Counting…
        </p>
      ) : stats === "error" ? (
        <p className="stats-dialog__empty" role="alert">
          Couldn't load your stats.
        </p>
      ) : stats.plays === 0 ? (
        <p className="stats-dialog__empty">You haven't played this {KIND_WORD[subject.kind]} yet.</p>
      ) : (
        <StatsStripView stats={stats} kind={subject.kind} sparkWidth={480} />
      )}
      <div className="stats-dialog__actions">
        {statsPage && (
          <Button
            variant="ghost"
            onClick={() => {
              onClose();
              nav.go({ name: "stats" });
            }}
          >
            All your stats
          </Button>
        )}
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      </div>
    </div>
  );
}

export function StatsDialogHost() {
  const subject = useStatsDialog((s) => s.subject);
  const close = useStatsDialog((s) => s.close);
  return (
    <Dialog open={!!subject} onClose={close} label="Your stats">
      {subject && <StatsBody key={`${subject.kind}:${subject.id}`} subject={subject} onClose={close} />}
    </Dialog>
  );
}
