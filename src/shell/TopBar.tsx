import { Check, ChevronLeft, ChevronRight, Copy, House, Minus, Square, X } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Tooltip } from "../components/Tooltip";
import { artUrl, backend } from "../lib/backend";
import { chrome, modKey } from "../lib/platform";
import { useApp } from "../stores/app";
import { nav, useCanGoBack, useCanGoForward, useCurrentEntry } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { createPlaylist } from "../stores/playlists";
import { toast, useToasts } from "../stores/toasts";
import { Logo } from "./Logo";
import { SearchBox } from "./SearchBox";
import "./topbar.css";

/** Below this window width the search pill becomes a round button beside Home. */
export const TOPBAR_COLLAPSE_WIDTH = 860;

type AppWindow = {
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<void>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  onResized: (cb: () => void) => Promise<() => void>;
};

function useWindowWidth() {
  const [w, setW] = useState(() => (typeof window !== "undefined" ? window.innerWidth : 1280));
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

/** Minimise / maximise / close for the undecorated Windows window. */
function WindowControls() {
  const [win, setWin] = useState<AppWindow | null>(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let un: (() => void) | undefined;
    let alive = true;
    import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
      const w = getCurrentWindow() as unknown as AppWindow;
      if (!alive) return;
      setWin(w);
      setMaximized(await w.isMaximized());
      un = await w.onResized(async () => setMaximized(await w.isMaximized()));
    });
    return () => {
      alive = false;
      un?.();
    };
  }, []);

  return (
    <div className="topbar__controls">
      <button type="button" className="topbar__winbtn" aria-label="Minimize" onClick={() => win?.minimize()}>
        <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
      <button type="button" className="topbar__winbtn" aria-label={maximized ? "Restore" : "Maximize"} onClick={() => win?.toggleMaximize()}>
        {maximized ? <Copy size={13} strokeWidth={1.5} aria-hidden="true" /> : <Square size={13} strokeWidth={1.5} aria-hidden="true" />}
      </button>
      <button type="button" className="topbar__winbtn topbar__winbtn--close" aria-label="Close" onClick={() => win?.close()}>
        <X size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
    </div>
  );
}

/**
 * A bare drag strip with the window buttons, for screens without the top
 * bar (sign-in). Nothing in a plain browser.
 */
export function WindowBar() {
  if (chrome === "none") return null;
  return (
    <div className={`windowbar windowbar--${chrome}`} data-tauri-drag-region>
      {chrome === "windows" && <WindowControls />}
    </div>
  );
}

function checkForUpdates() {
  toast.info("Checking for updates…");
  backend.updateCheck().then(
    (u) => {
      if (!u.available) return toast.info("You're up to date.");
      useToasts.getState().push(
        {
          kind: "info",
          message: u.version ? `Platter ${u.version} is ready. Restart to install it.` : "A new version is ready. Restart to install it.",
          action: { label: "Restart", run: () => void backend.updateInstallOnRestart() },
        },
        0,
      );
    },
    () => toast.error("Couldn't check for updates"),
  );
}

/**
 * A round face: the Plex profile picture when there is one, over the
 * fallback (the username's first letter), which shows while the picture
 * loads or when it fails.
 */
function AvatarFace({ path, size, fallback, className }: { path: string | null; size: number; fallback: ReactNode; className: string }) {
  const src = artUrl(path, size);
  const [loaded, setLoaded] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const showImg = !!src && failed !== src;
  return (
    <span aria-hidden="true" className={className} data-avatar={showImg && loaded === src ? "image" : "letter"}>
      {fallback}
      {showImg && (
        <img
          className={`avatar-img ${loaded === src ? "is-loaded" : ""}`}
          src={src}
          alt=""
          draggable={false}
          onLoad={() => setLoaded(src)}
          onError={() => setFailed(src)}
        />
      )}
    </span>
  );
}

/** The avatar button and its app menu. */
export function AppMenu() {
  const username = useApp((s) => s.auth?.username ?? null);
  const showAvatar = useSettings((s) => s.settings?.showPlexAvatar !== false);
  const avatar = useApp((s) => (showAvatar ? (s.auth?.avatar ?? null) : null));
  const privateSession = useSettings((s) => !!s.settings?.privateSession);
  const [open, setOpen] = useState(false);
  const [confirmSignOut, setConfirmSignOut] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const viaKeyboard = useRef(false);

  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    if (viaKeyboard.current) menu.current?.querySelector<HTMLElement>('[role^="menuitem"]')?.focus();
    else menu.current?.focus();
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!menu.current?.contains(t) && !button.current?.contains(t)) close(false);
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  const run = (fn: () => void) => () => {
    close();
    fn();
  };

  const onMenuKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(menu.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? [])];
    if (!items.length) return;
    const at = items.indexOf(document.activeElement as HTMLElement);
    let next = -1;
    if (e.key === "ArrowDown") next = at < 0 ? 0 : (at + 1) % items.length;
    else if (e.key === "ArrowUp") next = at <= 0 ? items.length - 1 : at - 1;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = items.length - 1;
    else if (e.key === "Tab") return close(false);
    if (next >= 0) {
      e.preventDefault();
      items[next].focus();
    }
  };

  const initial = (username ?? "").trim().charAt(0).toUpperCase();

  return (
    <div className="appmenu">
      <Tooltip label={username ? `${username}` : "Account and app"} placement="bottom">
        <button
          ref={button}
          type="button"
          className={`topbar__avatar ${open ? "is-open" : ""}`}
          aria-label={username ? `App menu, signed in as ${username}` : "App menu"}
          aria-haspopup="menu"
          aria-expanded={open}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") viaKeyboard.current = true;
          }}
          onClick={() => {
            if (open) return close();
            setOpen(true);
          }}
          onPointerDown={() => (viaKeyboard.current = false)}
        >
          <AvatarFace path={avatar} size={32} className="topbar__avatar-face" fallback={initial || <Logo size={20} />} />
        </button>
      </Tooltip>
      {open && (
        <div className="appmenu__panel" role="menu" aria-label="App menu" ref={menu} tabIndex={-1} onKeyDown={onMenuKey}>
          {username && (
            <div className="appmenu__who" aria-hidden="true">
              <AvatarFace path={avatar} size={28} className="appmenu__who-face" fallback={initial} />
              <span className="appmenu__who-name">{username}</span>
            </div>
          )}
          <button type="button" role="menuitem" className="appmenu__item" onClick={run(() => void createPlaylist())}>
            <span>Create playlist</span>
          </button>
          <div className="appmenu__sep" role="separator" />
          <button type="button" role="menuitem" className="appmenu__item" onClick={run(() => nav.go({ name: "settings" }))}>
            <span>Settings</span>
            <kbd className="appmenu__hint">{modKey}+,</kbd>
          </button>
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={privateSession}
            className="appmenu__item"
            onClick={run(() => void updateSettings({ privateSession: !privateSession }))}
          >
            <span>Private session</span>
            {privateSession && <Check size={16} className="appmenu__check" aria-hidden="true" />}
          </button>
          <button type="button" role="menuitem" className="appmenu__item" onClick={run(checkForUpdates)}>
            <span>Check for updates</span>
          </button>
          <div className="appmenu__sep" role="separator" />
          <button type="button" role="menuitem" className="appmenu__item" onClick={run(() => setConfirmSignOut(true))}>
            <span>Sign out</span>
          </button>
        </div>
      )}
      <Dialog open={confirmSignOut} onClose={() => setConfirmSignOut(false)} label="Sign out of Plex?" className="confirm">
        <h2 className="confirm__title">Sign out of Plex?</h2>
        <p className="confirm__body">Platter forgets your Plex sign-in on this computer. You'll need to sign in again to play your music.</p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setConfirmSignOut(false)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              setConfirmSignOut(false);
              backend.authSignOut().catch(() => toast.error("Couldn't sign out"));
            }}
          >
            Sign out
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

/**
 * The top bar, which is also the window's drag area: back and forward on the
 * left, Home and the search pill centred on the window, the app menu on the
 * right (then the window buttons on Windows; traffic lights sit at the left
 * on macOS).
 */
export function TopBar() {
  const width = useWindowWidth();
  const narrow = width < TOPBAR_COLLAPSE_WIDTH;
  const canBack = useCanGoBack();
  const canForward = useCanGoForward();
  const onHome = useCurrentEntry().route.name === "home";

  return (
    <header className={`topbar topbar--${chrome} ${narrow ? "is-narrow" : ""}`} data-tauri-drag-region>
      <div className="topbar__start" data-tauri-drag-region>
        {chrome !== "mac" && (
          <span className="topbar__logo" data-tauri-drag-region>
            <Logo size={24} />
          </span>
        )}
        <IconButton
          className="topbar__nav"
          label="Go back"
          tooltip="Go back (Alt+←)"
          tooltipPlacement="bottom"
          icon={<ChevronLeft size={22} />}
          disabled={!canBack}
          onClick={() => nav.back()}
        />
        <IconButton
          className="topbar__nav"
          label="Go forward"
          tooltip="Go forward (Alt+→)"
          tooltipPlacement="bottom"
          icon={<ChevronRight size={22} />}
          disabled={!canForward}
          onClick={() => nav.forward()}
        />
      </div>
      <div className="topbar__centre" data-tauri-drag-region>
        <Tooltip label="Home" placement="bottom">
          <button
            type="button"
            className={`topbar__round topbar__home ${onHome ? "is-active" : ""}`}
            aria-label="Home"
            aria-current={onHome ? "page" : undefined}
            onClick={() => nav.go({ name: "home" })}
          >
            <House size={22} aria-hidden="true" fill={onHome ? "currentColor" : "none"} />
          </button>
        </Tooltip>
        <SearchBox collapsed={narrow} />
      </div>
      <div className="topbar__end" data-tauri-drag-region>
        <AppMenu />
        {chrome === "windows" && <WindowControls />}
      </div>
    </header>
  );
}
