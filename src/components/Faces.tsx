import { useState } from "react";
import { artUrl } from "../lib/backend";
import type { CirclePerson } from "../lib/types";
import "./circle.css";

/** One round face: the person's picture, else their name's letter. */
export function Face({ person, size = 24, className = "" }: { person: Pick<CirclePerson, "name" | "avatar">; size?: number; className?: string }) {
  const src = person.avatar ? artUrl(person.avatar, size * 2) : null;
  const [loaded, setLoaded] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const showImg = !!src && failed !== src;
  const letter = person.name.trim().charAt(0).toUpperCase() || "?";
  return (
    <span aria-hidden="true" className={`face ${className}`} style={{ width: size, height: size, fontSize: Math.max(10, Math.round(size * 0.42)) }} data-avatar={showImg && loaded === src ? "image" : "letter"}>
      {letter}
      {showImg && (
        <img className={`avatar-img ${loaded === src ? "is-loaded" : ""}`} src={src} alt="" draggable={false} onLoad={() => setLoaded(src)} onError={() => setFailed(src)} />
      )}
    </span>
  );
}

/** Up to `max` faces overlapping, the first on top. */
export function Faces({ people, max = 3, size = 24 }: { people: Pick<CirclePerson, "id" | "name" | "avatar">[]; max?: number; size?: number }) {
  const shown = people.slice(0, max);
  return (
    <span className="faces" aria-hidden="true">
      {shown.map((p, i) => (
        <span key={p.id} className="faces__slot" style={{ zIndex: max - i }}>
          <Face person={p} size={size} />
        </span>
      ))}
    </span>
  );
}
