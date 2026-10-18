/**
 * The transport strip: Platter's player for every frame but Classic. Small
 * cover, title and artist, the transport, the tonearm seek line with its
 * times and format in engraved plates, then volume, output and the panels.
 * It is built from the player bar's own pieces, so every control, key and
 * accessible name is the same and keyboard and screen-reader use don't
 * change from one frame to the next. Brass marks only what's on.
 */
import { MonitorSpeaker } from "lucide-react";
import { Fragment } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { IconButton } from "../components/Button";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { Skeleton } from "../components/Skeleton";
import { trackMenuItems } from "../components/TrackRow";
import { trackArtistLine } from "../lib/format";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { useUi } from "../stores/ui";
import { HifiMark } from "./HifiMark";
import { ListenMark } from "./ListenStrip";
import {
  ExpandButton,
  LyricsButton,
  MiniplayerButton,
  MoreButton,
  NowPlayingAnnouncer,
  PlayControls,
  PrivateSession,
  QueueButton,
  SeekBar,
  SuggestionButtons,
  Volume,
} from "./PlayerBar";
import { PlayerSubline } from "./PlayerExtras";
import { SignalPanel } from "./SignalPath";
import "./transport-strip.css";

export type StripPlace = "top" | "bottom" | "left";

/** The cover, title and artist at the strip's start. */
function StripNow() {
  const state = usePlayer((s) => s.state);
  const expanded = useFullWindow((s) => s.open);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen) && !expanded;
  const buttons = useLayout().playerButtons;
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const track = state?.track;
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  const dragNow =
    track && songDrags
      ? { draggable: true, onDragStart: (e: React.DragEvent) => void startSongDrag(e, tracksDrag([track])), onDragEnd: endSongDrag }
      : null;
  return (
    <div className="ts-now" role="region" aria-label={nowText ?? "Nothing playing"} {...dragNow}>
      {track ? (
        <>
          <button
            type="button"
            className="ts-now__art"
            aria-label="Now playing view"
            aria-pressed={nowPlayingOpen}
            onClick={(e) => e.detail < 2 && useUi.getState().toggleNowPlaying()}
            onDoubleClick={() => {
              useUi.getState().toggleNowPlaying();
              fullWindow.open();
            }}
          >
            <Art path={track.thumb} size={48} />
          </button>
          <div className="ts-now__meta">
            <div
              className="ts-now__title truncate"
              onContextMenu={(e) => {
                e.preventDefault();
                openContextMenu(`Options for ${track.title}`, trackMenuItems(track), e);
              }}
            >
              <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
              <ExplicitMark trackId={track.id} />
            </div>
            <PlayerSubline track={track} />
            <ListenMark />
          </div>
          <SuggestionButtons />
          {buttons.includes("like") && <SaveButton track={track} place="bar" />}
        </>
      ) : state === null ? (
        // Still loading the restored session: the same shapes, so nothing moves.
        <>
          <Skeleton width={48} height={48} />
          <div className="ts-now__meta" style={{ flex: 1 }}>
            <Skeleton width="70%" height={13} />
            <Skeleton width="45%" height={11} style={{ marginTop: 6 }} />
          </div>
        </>
      ) : (
        <p className="ts-now__empty">Nothing playing. Pick a record.</p>
      )}
    </div>
  );
}

/** Volume, output and the panels, in the order the layout's button list gives. */
function StripExtras() {
  const outputOpen = useUi((s) => s.outputOpen);
  const toggleOutput = useUi((s) => s.toggleOutput);
  const buttons = useLayout().playerButtons;
  const hasTrack = usePlayer((s) => !!s.state?.track);
  return (
    <div className="ts-extras">
      <PrivateSession />
      {!buttons.includes("devices") && <HifiMark />}
      {buttons.map((id) => (
        <Fragment key={id}>
          {id === "lyrics" ? (
            <LyricsButton />
          ) : id === "queue" ? (
            <QueueButton />
          ) : id === "devices" ? (
            <>
              <HifiMark />
              <IconButton label="Connect to a device" pressed={outputOpen} icon={<MonitorSpeaker size={18} />} onClick={toggleOutput} />
            </>
          ) : id === "volume" ? (
            <Volume />
          ) : id === "miniplayer" ? (
            <span className="player-bar__fold">
              <MiniplayerButton />
            </span>
          ) : id === "fullWindow" ? (
            <span className="player-bar__fold">
              <ExpandButton disabled={!hasTrack} />
            </span>
          ) : null}
        </Fragment>
      ))}
      <MoreButton buttons={buttons} hasTrack={hasTrack} />
    </div>
  );
}

/**
 * The strip itself. `showNow` puts the cover and title in it (the frame
 * placed now playing beside the transport); `place` says which edge it's on.
 */
export function TransportStrip({ place, showNow = true }: { place: StripPlace; showNow?: boolean }) {
  const track = usePlayer((s) => s.state?.track);
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  return (
    <aside className={`transport-strip transport-strip--${place} ${showNow ? "" : "transport-strip--bare"}`} aria-label="Player" data-testid="transport-strip">
      <NowPlayingAnnouncer text={nowText} />
      {showNow && <StripNow />}
      <div className="ts-deck">
        <PlayControls />
      </div>
      <div className="ts-line">
        <SeekBar strip />
      </div>
      <StripExtras />
      <SignalPanel />
    </aside>
  );
}
