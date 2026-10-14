import { ChevronLeft, Ellipsis, Maximize2, Minimize2, PanelRightClose, Play } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink, useTrackCredits } from "../components/ArtistLinks";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { trackMenuItems } from "../components/TrackRow";
import type { NowPlayingDetails, Track } from "../lib/types";
import { useApp } from "../stores/app";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useUi } from "../stores/ui";
import {
  buildCredits,
  contextRoute,
  CREDITS_PREVIEW,
  creditsWorthShowing,
  lyricsPreview,
  nextInQueue,
  useCoverColors,
  useNowPlayingDetails,
} from "./nowPlaying";

/** Requested art size for the panel's cover: the widest panel minus its padding. */
const PANEL_ART = 388;
/** Requested art size for the expanded view's cover. */
const EXPANDED_ART = 640;

type View = "main" | "lyrics" | "bio";

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
  const rows = buildCredits(trackCredits, details?.credits ?? [], lookup);
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
            <Art path={next.track.thumb} size={48} />
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

/** Title and artists under the cover. The saved tick joins when likes arrive. */
function TrackTitle({ track, large }: { track: Track; large?: boolean }) {
  return (
    <div className={`np-title ${large ? "np-title--large" : ""}`}>
      <div className="np-title__name truncate">
        <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
      </div>
      <ArtistLinks track={track} className="np-title__artists truncate" />
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
  const colors = useCoverColors(track.thumb);
  const lyrics = lyricsOn && details?.lyrics?.lines.length ? details.lyrics : null;
  const about = details?.artist?.summary ? details.artist : null;

  if (view === "lyrics" && lyrics) {
    return (
      <SubView title={`Lyrics: ${track.title}`} onBack={() => setView("main")}>
        <div className="np-lyrics-full">
          {lyrics.lines.map((l, i) => (l ? <p key={i}>{l}</p> : <p key={i} className="np-lyrics-full__gap" aria-hidden="true" />))}
        </div>
      </SubView>
    );
  }
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
      <button type="button" className="np-art" aria-label={`Go to album ${track.albumTitle}`} onClick={() => nav.go({ name: "album", id: track.albumId })}>
        <Art path={track.thumb} fluid requestSize={PANEL_ART} />
      </button>
      <TrackTitle track={track} />
      {lyrics && <LyricsCard lines={lyrics.lines} tint={colors?.shade ?? null} onOpen={() => setView("lyrics")} />}
      {about && <AboutCard details={about} onOpen={() => setView("bio")} />}
      <CreditsCard track={track} details={details} />
      <NextCard />
    </div>
  );
}

/** Right panel: the playing track with its cover, lyrics, artist, credits and what's next. */
export function NowPlayingPanel() {
  const track = usePlayer((s) => s.state?.track);
  const expanded = useUi((s) => s.nowPlayingExpanded);
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
            onClick={() => useUi.getState().setNowPlayingExpanded(true)}
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

/** The now-playing view over the whole main area, on a colour from the cover. */
export function NowPlayingExpanded() {
  const track = usePlayer((s) => s.state?.track);
  const colors = useCoverColors(track?.thumb);
  const minimize = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    minimize.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      // Esc leaves, unless a menu or dialog above us takes it first.
      if (e.key !== "Escape" || e.defaultPrevented || useUi.getState().menu || document.querySelector("dialog[open], [role=dialog]")) return;
      e.preventDefault();
      useUi.getState().setNowPlayingExpanded(false);
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      const expand = document.querySelector<HTMLElement>(".np-panel__expand");
      (expand ?? returnTo)?.focus?.();
    };
  }, []);

  const style = colors ? ({ "--np-bg": colors.vivid } as CSSProperties) : undefined;
  return (
    <section className="np-expanded" aria-label="Now playing, expanded" style={style}>
      <header className="np-expanded__head">
        <ContextName className="np-expanded__context" />
        <div className="np-panel__actions">
          {track && (
            <IconButton
              label={`More options for ${track.title}`}
              tooltip="More options"
              icon={<Ellipsis size={20} />}
              onClick={(e) => moreOptions(track, e.currentTarget)}
            />
          )}
          <IconButton
            ref={minimize}
            label="Leave expanded view"
            tooltip="Minimize (Esc)"
            tooltipPlacement="bottom"
            icon={<Minimize2 size={18} />}
            onClick={() => useUi.getState().setNowPlayingExpanded(false)}
          />
        </div>
      </header>
      {track ? (
        <div className="np-expanded__stage" key={track.id}>
          <div className="np-expanded__art">
            <Art path={track.thumb} fluid requestSize={EXPANDED_ART} />
          </div>
          <TrackTitle track={track} large />
        </div>
      ) : (
        <p className="np-empty">Nothing playing.</p>
      )}
    </section>
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
        if (ui.nowPlayingPrefs.openOnStart && !ui.queueOpen && !ui.nowPlayingOpen) ui.setNowPlayingOpen(true);
      }),
    [],
  );
}
