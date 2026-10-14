import { useId, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { cancelPassword, submitPassword, useRemote } from "../stores/remote";
import "./cast.css";

/** Asks once for a password-protected AirPlay speaker's password. It's kept in
 * the system keychain after it works. */
export function AirPlayPassword() {
  const ask = useRemote((s) => s.password);
  if (!ask) return null;
  // A fresh form for each speaker.
  return <PasswordForm key={ask.player.id} name={ask.player.name} error={ask.error} />;
}

function PasswordForm({ name, error }: { name: string; error: string | null }) {
  const [value, setValue] = useState("");
  const busy = useRemote((s) => s.busy !== undefined);
  const input = useRef<HTMLInputElement>(null);
  const id = useId();
  const title = `Password for ${name}`;
  return (
    <Dialog open onClose={cancelPassword} label={title} className="confirm airplay-pw" initialFocus={input}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (value) void submitPassword(value);
        }}
      >
        <h2 className="confirm__title">{title}</h2>
        <p className="confirm__body" id={`${id}-hint`}>
          This AirPlay speaker has a password. Platter keeps it in your computer's keychain so you only enter it once.
        </p>
        <label className="airplay-pw__label" htmlFor={`${id}-pw`}>
          AirPlay password
        </label>
        <input
          ref={input}
          id={`${id}-pw`}
          className="airplay-pw__input"
          type="password"
          autoComplete="off"
          value={value}
          aria-describedby={error ? `${id}-err` : `${id}-hint`}
          aria-invalid={error ? true : undefined}
          onChange={(e) => setValue(e.target.value)}
        />
        {error && (
          <p className="confirm__error" id={`${id}-err`} role="alert">
            {error}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" type="button" onClick={cancelPassword}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" disabled={!value || busy}>
            {busy ? "Connecting…" : "Connect"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
