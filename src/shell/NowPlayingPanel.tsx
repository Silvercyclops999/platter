import { ChevronLeft, Ellipsis, Maximize2, PanelRightClose, Play } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink, useTrackCredits } from "../components/ArtistLinks";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { trackMenuItems } from "../components/TrackRow";
import { isOffline } from "../lib/offline";
import type { NowPlayingDetails, Track } from "../lib/types";
import { songCredits, useAlbumCredits } from "../lib/useAlbumCredits";
import { useApp } from "../stores/app";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { rightView, useUi } from "../stores/ui";
import {
  buildCredits,
  withAlbumCredits,
  contextRoute,
  CREDITS_PREVIEW,
  creditsWorthShowing,
  lyricsPreview,
  nextInQueue,
  useCoverColors,
  useNowPlayingDetails,
} from "./nowPlaying";
import { timedLines, useLyrics } from "./lyrics";
import { useCoverTap } from "../visualizer/useVis";

/** Requested art size for the panel's cover: the widest panel minus its padding. */
const PANEL_ART = 388;

type View = "main" | "bio";

/** The context name in the header, linking to it when it's a page. */
function ContextName({ className = "" }: { className?: string }) {
  const ctx = usePlayer((s) => s.state?.queue.context);
  const track = usePlayer((s) => s.state?.track);
  const title = ctx?.title || (ctx?.kind === "album" ? track?.albumTitle : null) || "Now playing";
  const route = contextRoute(ctx);
  return (
    <span className={`np-context truncate ${className}`}>
      {route ? <TextLink onActivate={() => nav.go(route)}>{title}</TextLink> : title}
    </span>
  );
}

function moreOptions(track: Track, el: HTMLElement) {
  openContextMenu(`Options for ${track.title}`, trackMenuItems(track), el);
}

/** A rounded card with an optional heading row. */
function NpCard({ title, action, className = "", style, children }: { title?: ReactNode; action?: ReactNode; className?: string; style?: CSSProperties; children: ReactNode }) {
  return (
    <section className={`np-card ${className}`} style={style}>
      {(title || action) && (
        <div className="np-card__head">
          {title && <h3 className="np-card__title">{title}</h3>}
          {action}
        </div>
      )}
      {children}
    </section>
  );
}

function LyricsCard({ lines, tint, onOpen }: { lines: string[]; tint: string | null; onOpen: () => void }) {
  const preview = lyricsPreview(lines);
  return (
    <NpCard
      className="np-lyrics"
      style={tint ? ({ "--np-card-tint": tint } as CSSProperties) : undefined}
      title="Lyrics"
      action={
        <button type="button" className="np-card__action" onClick={onOpen}>
          Show lyrics
        </button>
      }
    >
      <div className="np-lyrics__preview" aria-hidden="true">
        {preview.map((l, i) => (
          <p key={i} className="np-lyrics__line">
            {l}
          </p>
        ))}
      </div>
    </NpCard>
  );
}

function AboutCard({ details, onOpen }: { details: NonNullable<NowPlayingDetails["artist"]>; onOpen: () => void }) {
  const photo = details.art || details.thumb;
  return (
    <section className="np-card np-about">
      <button type="button" className="np-about__open" aria-label={`About ${details.title}`} onClick={onOpen}>
        {photo && (
          <span className="np-about__photo">
            <Art path={photo} fluid requestSize={PANEL_ART} kind="artist" />
          </span>
        )}
        <span className={`np-about__heading ${photo ? "on-photo" : ""}`}>About the artist</span>
      </button>
      <div className="np-about__body">
        <TextLink className="np-about__name" onActivate={() => nav.go({ name: "artist", id: details.id })}>
          {details.title}
        </TextLink>
        <p className="np-about__bio">{details.summary}</p>
      </div>
    </section>
  );
}

function CreditsCard({ track, details }: { track: Track; details: NowPlayingDetails | null }) {
  const lookup = useApp((s) => s.artistLookup);
  const trackCredits = useTrackCredits(track);
  const [all, setAll] = useState(false);
  const album = useAlbumCredits(track.albumId);
  const rows = withAlbumCredits(buildCredits(trackCredits, details?.credits ?? [], lookup), songCredits(album, track.id));
  if (!creditsWorthShowing(rows)) return null;
  const shown = all ? rows : rows.slice(0, CREDITS_PREVIEW);
  return (
    <NpCard
      className="np-credits"
      title="Credits"
      action={
        rows.length > CREDITS_PREVIEW && (
          <button type="button" className="np-card__action" aria-expanded={all} onClick={() => setAll(!all)}>
            {all ? "Show less" : "Show all"}
          </button>
        )
      }
    >
      <ul className="np-credits__list">
        {shown.map((r) => (
          <li key={r.name} className="np-credits__row">
            <span className="np-credits__name truncate">
              {r.id ? <TextLink onActivate={() => nav.go({ name: "artist", id: r.id! })}>{r.name}</TextLink> : r.name}
            </span>
            <span className="np-credits__roles truncate">{r.roles.join(" • ")}</span>
          </li>
        ))}
      </ul>
    </NpCard>
  );
}

function NextCard() {
  const next = usePlayer((s) => nextInQueue(s.state?.queue));
  return (
    <NpCard
      className="np-next"
      title="Next in queue"
      action={
        <button type="button" className="np-card__action" onClick={() => useUi.getState().setQueueOpen(true)}>
          Open queue
        </button>
      }
    >
      {next ? (
        <div className="np-next__row" onDoubleClick={() => player.queueJump(next.itemId)}>
          <button type="button" className="np-next__art" aria-label={`Play ${next.track.title}`} onClick={() => player.queueJump(next.itemId)}>
            <Art path={next.track.thumb} sleeve={{ title: next.track.albumTitle, artist: next.track.artistTitle }} size={48} />
            <span className="np-next__play" aria-hidden="true">
              <Play size={18} fill="currentColor" />
            </span>
          </button>
          <div className="np-next__meta">
            <div className="np-next__title truncate">{next.track.title}</div>
            <ArtistLinks track={next.track} className="np-next__artist truncate" />
          </div>
        </div>
      ) : (
        <p className="np-card__empty">Nothing else queued.</p>
      )}
    </NpCard>
  );
}

/** Title and artists under the cover, with the like control after them. */
function TrackTitle({ track }: { track: Track }) {
  return (
    <div className="np-title">
      <div className="np-title__text">
        <div className="np-title__name truncate">
          <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
        </div>
        <ArtistLinks track={track} className="np-title__artists truncate" />
      </div>
      <SaveButton track={track} place="panel" />
    </div>
  );
}

/** Full lyrics or full bio, inside the panel, with a way back. */
function SubView({ title, onBack, children }: { title: string; onBack: () => void; children: ReactNode }) {
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => back.current?.focus(), []);
  return (
    <div className="np-sub">
      <div className="np-sub__head">
        <IconButton ref={back} label="Back to now playing" tooltip="Back" icon={<ChevronLeft size={18} />} onClick={onBack} />
        <h3 className="np-sub__title truncate">{title}</h3>
      </div>
      {children}
    </div>
  );
}

function PanelBody({ track, view, setView }: { track: Track; view: View; setView: (v: View) => void }) {
  const { details } = useNowPlayingDetails(track.id);
  const lyricsOn = useUi((s) => s.nowPlayingPrefs.lyricsCard);
  const coverTap = useCoverTap();
  const colors = useCoverColors(track.thumb);
  // The same lyrics the lyrics view shows (your pick, LRCLIB, your own, a
  // transcription), and no card when it has none to show.
  const { lyrics: found } = useLyrics(lyricsOn ? track.id : null);
  const lines = found ? (timedLines(found)?.map((l) => l.text) ?? found.lines) : [];
  const lyrics = lyricsOn && found && lines.some((l) => l.trim()) ? { lines } : null;
  // Offline: sections that need the server (about the artist, its credits) step aside.
  const offline = useApp((s) => isOffline(s.health));
  const about = !offline && details?.artist?.summary ? details.artist : null;
  // The spinning-record style turns the cover while music plays.
  const spinning = usePlayer((s) => s.state?.status === "playing");

  if (view === "bio" && about) {
    return (
      <SubView title={about.title} onBack={() => setView("main")}>
        <div className="np-bio-full">
          {(about.summary ?? "").split(/\n\s*\n/).map((p, i) => (
            <p key={i}>{p}</p>
          ))}
        </div>
      </SubView>
    );
  }

  return (
    <div className="np-content" key={track.id}>
      {/* The cover opens the visualizer (the title goes to the album); with that off it goes to the album. */}
      <button
        type="button"
        className={`np-art ${spinning ? "is-spinning" : ""}`}
        aria-label={coverTap ? "Open the visualizer" : `Go to album ${track.albumTitle}`}
        onClick={() => (coverTap ? fullWindow.open("visualizer") : nav.go({ name: "album", id: track.albumId }))}
      >
        <Art path={track.thumb} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} fluid requestSize={PANEL_ART} />
      </button>
      <TrackTitle track={track} />
      {/* Show lyrics opens the lyrics view itself: synced, with its tools. */}
      {lyrics && <LyricsCard lines={lyrics.lines} tint={colors?.shade ?? null} onOpen={() => fullWindow.open("lyrics")} />}
      {about && <AboutCard details={about} onOpen={() => setView("bio")} />}
      <CreditsCard track={track} details={offline ? null : details} />
      <NextCard />
    </div>
  );
}

/** Right panel: the playing track with its cover, lyrics, artist, credits and what's next. */
export function NowPlayingPanel() {
  const track = usePlayer((s) => s.state?.track);
  const expanded = useFullWindow((s) => s.open);
  const [scrolled, setScrolled] = useState(false);
  const [view, setView] = useState<{ id: string | undefined; v: View }>({ id: track?.id, v: "main" });
  // Full lyrics or bio belong to one track: a new track goes back to the main view.
  const current: View = view.id === track?.id ? view.v : "main";
  const close = () => useUi.getState().setNowPlayingOpen(false);

  return (
    <aside className="np-panel" aria-label="Now playing">
      <header className={`np-panel__head ${scrolled ? "is-scrolled" : ""}`}>
        <IconButton className="np-panel__hide" label="Hide now playing view" tooltip="Hide" size="sm" icon={<PanelRightClose size={16} />} onClick={close} />
        <ContextName />
        <div className="np-panel__actions">
          {track && (
            <IconButton
              label={`More options for ${track.title}`}
              tooltip="More options"
              icon={<Ellipsis size={18} />}
              onClick={(e) => moreOptions(track, e.currentTarget)}
            />
          )}
          <IconButton
            className="np-panel__expand"
            label="Expand now playing view"
            tooltip="Expand"
            pressed={expanded}
            icon={<Maximize2 size={16} />}
            onClick={() => fullWindow.open("artwork")}
          />
        </div>
      </header>
      <div className="np-panel__body" onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 0)}>
        {track ? (
          <PanelBody track={track} view={current} setView={(v) => setView({ id: track.id, v })} />
        ) : (
          <p className="np-empty">Nothing playing. Pick something and it shows up here.</p>
        )}
      </div>
    </aside>
  );
}

const isPlaying = (s: string | undefined) => s === "playing" || s === "loading" || s === "buffering";

/**
 * "Open now playing when a song starts": a new track that starts playing
 * opens the panel, as long as the right slot is free (an open queue stays).
 * The session restored at launch doesn't count.
 */
export function useOpenNowPlayingOnStart() {
  useEffect(
    () =>
      usePlayer.subscribe((s, prev) => {
        const id = s.state?.track?.id;
        if (!id || !prev.state || id === prev.state.track?.id || !isPlaying(s.state?.status)) return;
        const ui = useUi.getState();
        // The layout's Now playing option "Off" keeps the panel closed until asked for.
        if (useSettings.getState().settings?.layout?.nowPlaying === "off") return;
        // Never over the expanded view: opening the panel would close it.
        if (ui.nowPlayingPrefs.openOnStart && !rightView(ui) && !useFullWindow.getState().open) ui.setNowPlayingOpen(true);
      }),
    [],
  );
}
