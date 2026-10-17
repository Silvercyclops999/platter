/**
 * Create a Blend: pick people to combine tastes with, up to ten of you.
 * People in your Plex Home and on your server are invited directly; a link
 * covers anyone else.
 */
import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Face } from "../components/Faces";
import { backend } from "../lib/backend";
import type { BlendCreated, CircleAccount } from "../lib/types";
import { nav } from "../stores/router";
import { copyText, toast } from "../stores/toasts";
import { failText } from "../lib/errorText";

/** People in one Blend, you included. */
export const BLEND_MAX = 10;

export function BlendCreatePage() {
  const [people, setPeople] = useState<CircleAccount[] | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [made, setMade] = useState<BlendCreated | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let alive = true;
    backend.circlePeople().then(
      (p) => alive && setPeople(p),
      () => alive && setPeople([]),
    );
    return () => {
      alive = false;
    };
  }, []);
  const full = picked.length + 1 >= BLEND_MAX;
  const toggle = (key: string) => setPicked((p) => (p.includes(key) ? p.filter((k) => k !== key) : full ? p : [...p, key]));
  const create = async () => {
    setBusy(true);
    try {
      setMade(await backend.blendCreate(picked));
      toast.info(picked.length ? `Invited ${picked.length === 1 ? "1 person" : `${picked.length} people`}` : "Your Blend is ready to share");
    } catch (e) {
      toast.error(failText(`Couldn't make the Blend`, e));
    } finally {
      setBusy(false);
    }
  };
  const group = (label: string, list: CircleAccount[]) =>
    list.length > 0 && (
      <section aria-label={label}>
        <h2 className="invite__heading">{label}</h2>
        <ul className="blend-new__people">
          {list.map((p) => {
            const on = picked.includes(p.key);
            return (
              <li key={p.key}>
                <button type="button" className="blend-new__person" aria-pressed={on} disabled={!on && full} onClick={() => toggle(p.key)}>
                  <Face person={p} size={64} />
                  <span className="truncate">{p.name}</span>
                  <span className={`blend-new__tick ${on ? "" : "blend-new__tick--off"}`} aria-hidden="true">
                    {on && <Check size={14} />}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </section>
    );
  return (
    <div className="blend-new">
      <h1 className="blend-new__title">Create a Blend</h1>
      <p className="blend-new__lede">
        Combine friends' tastes into one playlist, made fresh each day. Each of you gets songs you can play from your own library, and only a short summary of what you play most is shared.
      </p>
      {made ? (
        <>
          <div className="blend-new__link">
            <code>{made.invite.link}</code>
            <Button variant="secondary" icon={<Copy size={16} />} onClick={() => copyText(made.invite.link)}>
              Copy invite link
            </Button>
          </div>
          <p className="blend-new__lede">Send the link to anyone else. It works once, for one person, and runs out after a while.</p>
          <div className="blend-new__bar">
            <Button variant="primary" onClick={() => nav.go({ name: "playlist", id: made.id })}>
              Open Blend
            </Button>
          </div>
        </>
      ) : (
        <>
          {people === null ? (
            <p className="invite__note" role="status">
              Looking for people on your server…
            </p>
          ) : people.length === 0 ? (
            <p className="invite__note">Nobody else is on your server or Plex Home. Create the Blend and send its invite link.</p>
          ) : (
            <>
              {group("Your Plex Home", people.filter((p) => p.home))}
              {group("On your server", people.filter((p) => !p.home))}
            </>
          )}
          <div className="blend-new__bar">
            <span className="blend-new__count" role="status">
              {picked.length + 1} of {BLEND_MAX} people, you included{full ? ". That's the most a Blend takes." : ""}
            </span>
            <Button variant="primary" disabled={busy} onClick={() => void create()}>
              {picked.length ? "Create Blend" : "Create and get a link"}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
