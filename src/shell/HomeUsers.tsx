import { Lock } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import type { AppError, HomeUser, HomeUsers } from "../lib/types";
import { useCap } from "../stores/caps";
import { beginSwitch, switchDone, switchFailed } from "../stores/switching";
import { AvatarFace } from "./TopBar";

const NO_HOME: HomeUsers = { users: [], remember: true };

/** The account's Plex Home users, loaded when `open` (the app menu, Settings). */
export function useHomeUsers(open: boolean): HomeUsers {
  const [home, setHome] = useState<HomeUsers>(NO_HOME);
  // Only servers with Home users have any.
  const allowed = useCap("homeUsers");
  useEffect(() => {
    if (!open || !allowed) return;
    let alive = true;
    backend.homeUsers().then(
      (h) => alive && setHome(h),
      () => alive && setHome(NO_HOME),
    );
    return () => {
      alive = false;
    };
  }, [open, allowed]);
  return allowed ? home : NO_HOME;
}

/** Switch now, or ask for the PIN first when the user has one. */
export function switchHomeUser(user: HomeUser, askPin: (u: HomeUser) => void) {
  if (user.current) return;
  if (user.protected) return askPin(user);
  // The switching screen covers the window until the app restarts as them.
  beginSwitch(user);
  backend.homeSwitch(user.uuid).then(switchDone, (e: AppError) => switchFailed(e.message));
}

/**
 * The other Home users in the app menu, under the current one: one row
 * each with their picture; a lock marks those with a PIN.
 */
export function HomeUserItems({ users, onPick }: { users: HomeUser[]; onPick: (u: HomeUser) => void }) {
  const others = users.filter((u) => !u.current);
  if (users.length < 2 || !others.length) return null;
  return (
    <div role="group" aria-label="Switch Plex Home user" className="appmenu__users">
      {others.map((u) => (
        <button
          key={u.uuid}
          type="button"
          role="menuitem"
          aria-label={`Switch to ${u.name}${u.protected ? ", needs a PIN" : ""}`}
          className="appmenu__item appmenu__user"
          onClick={() => onPick(u)}
        >
          <span className="appmenu__user-who">
            <AvatarFace path={u.avatar} size={24} className="appmenu__user-face" fallback={u.name.trim().charAt(0).toUpperCase()} />
            <span className="appmenu__user-name">{u.name}</span>
          </span>
          {u.protected && <Lock size={14} className="appmenu__hint" aria-hidden="true" />}
        </button>
      ))}
      <div className="appmenu__sep" role="separator" />
    </div>
  );
}

/** Asks for a Home user's PIN; the PIN goes to plex.tv once and is never kept. */
export function HomePinDialog({ user, onClose }: { user: HomeUser | null; onClose: () => void }) {
  const [pin, setPin] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const id = useId();
  useEffect(() => {
    setPin("");
    setError(null);
    setBusy(false);
  }, [user]);
  if (!user) return null;
  const submit = () => {
    if (!/^\d{4}$/.test(pin)) return setError("A PIN is 4 digits.");
    setBusy(true);
    setError(null);
    beginSwitch(user);
    backend.homeSwitch(user.uuid, pin).then(
      () => {
        onClose();
        switchDone();
      },
      (e: AppError) => {
        // Back to the dialog as it was, to try again.
        switchFailed(e.message);
        setBusy(false);
        setPin("");
        setError(e.message || "Couldn't switch users");
        input.current?.focus();
      },
    );
  };
  return (
    <Dialog open onClose={onClose} label={`Enter ${user.name}'s PIN`} className="confirm home-pin" initialFocus={input}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <h2 className="confirm__title">Enter {user.name}'s PIN</h2>
        <p className="confirm__body" id={`${id}-hint`}>
          Platter switches to {user.name}'s music, likes and settings. The PIN goes to Plex and isn't saved.
        </p>
        <input
          ref={input}
          className="home-pin__input"
          type="password"
          inputMode="numeric"
          autoComplete="off"
          maxLength={4}
          pattern="[0-9]*"
          aria-label="PIN"
          aria-describedby={`${id}-hint`}
          aria-invalid={!!error}
          value={pin}
          disabled={busy}
          onChange={(e) => {
            setPin(e.target.value.replace(/\D/g, "").slice(0, 4));
            setError(null);
          }}
        />
        {error && (
          <p className="confirm__error" role="alert">
            {error}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" disabled={busy || pin.length !== 4}>
            Switch
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
