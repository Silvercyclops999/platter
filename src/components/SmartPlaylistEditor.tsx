/**
 * The smart playlist editor, mounted once in the shell: match all or any of
 * a list of rules, a limit, a sort and an update schedule, with a live count
 * of the songs the rules find. With an AI provider set, a prompted playlist
 * starts from a description that fills the rules and picks songs.
 */
import { Plus, Sparkles, X } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { AppError, SmartField, SmartOp, SmartPreview, SmartRule, SmartRules, SmartSort, SmartUpdate } from "../lib/types";
import { closeSmartEditor, isPlatterSmart, useCurate, type SmartEditorState } from "../stores/curate";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { Art } from "./Art";
import { Button, IconButton } from "./Button";
import { Dialog } from "./Dialog";
import { Select, type SelectOption } from "./Select";
import "./smart-editor.css";

type Kind = "tag" | "text" | "number" | "decade" | "date" | "bool" | "choice";

export const FIELDS: { value: SmartField; label: string; kind: Kind; hint?: string }[] = [
  { value: "genre", label: "Genre", kind: "tag", hint: "Rock" },
  { value: "style", label: "Style", kind: "tag", hint: "Shoegaze" },
  { value: "mood", label: "Mood", kind: "tag", hint: "Mellow" },
  { value: "artist", label: "Artist", kind: "text" },
  { value: "album", label: "Album", kind: "text" },
  { value: "year", label: "Year", kind: "number", hint: "1999" },
  { value: "decade", label: "Decade", kind: "decade" },
  { value: "addedAt", label: "Added", kind: "date" },
  { value: "playCount", label: "Plays", kind: "number", hint: "0" },
  { value: "lastPlayed", label: "Last played", kind: "date" },
  { value: "playedHere", label: "Played on this computer", kind: "date" },
  { value: "rating", label: "Rating (stars)", kind: "number", hint: "4" },
  { value: "liked", label: "Liked", kind: "bool" },
  { value: "duration", label: "Length (minutes)", kind: "number", hint: "5" },
  { value: "releaseType", label: "Release type", kind: "choice" },
  { value: "explicit", label: "Explicit", kind: "bool" },
  { value: "version", label: "Version", kind: "text", hint: "Remaster" },
];

const OPS: Record<Kind, { value: SmartOp; label: string }[]> = {
  tag: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
  ],
  text: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
    { value: "contains", label: "contains" },
    { value: "notContains", label: "doesn't contain" },
  ],
  number: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
    { value: "gt", label: "is more than" },
    { value: "lt", label: "is less than" },
  ],
  decade: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
  ],
  date: [
    { value: "inLast", label: "in the last" },
    { value: "notInLast", label: "not in the last" },
  ],
  bool: [{ value: "is", label: "is" }],
  choice: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
  ],
};

const SPANS: SelectOption<string>[] = [
  { value: "1d", label: "day" },
  { value: "7d", label: "7 days" },
  { value: "2w", label: "2 weeks" },
  { value: "30d", label: "30 days" },
  { value: "3m", label: "3 months" },
  { value: "6m", label: "6 months" },
  { value: "1y", label: "year" },
  { value: "2y", label: "2 years" },
];

const RELEASE_TYPES: SelectOption<string>[] = [
  { value: "albums", label: "Album" },
  { value: "singlesEps", label: "Single or EP" },
  { value: "live", label: "Live" },
  { value: "compilations", label: "Compilation" },
  { value: "soundtracks", label: "Soundtrack" },
  { value: "remixes", label: "Remix" },
  { value: "demos", label: "Demo" },
  { value: "mixtapes", label: "Mixtape" },
];

const DECADES: SelectOption<string>[] = Array.from({ length: 11 }, (_, i) => {
  const y = 1920 + i * 10;
  return { value: String(y), label: `${y}s` };
}).reverse();

const SORTS: SelectOption<SmartSort>[] = [
  { value: "none", label: "Library order" },
  { value: "random", label: "Random" },
  { value: "mostPlayed", label: "Most played" },
  { value: "leastPlayed", label: "Least played" },
  { value: "recentlyAdded", label: "Recently added" },
  { value: "recentlyPlayed", label: "Recently played" },
  { value: "rating", label: "Rating" },
];

const UPDATES: SelectOption<SmartUpdate>[] = [
  { value: "live", label: "Always up to date" },
  { value: "daily", label: "Daily" },
  { value: "weekly", label: "Weekly" },
];

type LimitChoice = "none" | "songs" | "hours";
const LIMITS: SelectOption<LimitChoice>[] = [
  { value: "none", label: "No limit" },
  { value: "songs", label: "Limit to songs" },
  { value: "hours", label: "Limit to hours" },
];

export const fieldKind = (f: SmartField): Kind => FIELDS.find((x) => x.value === f)?.kind ?? "text";

/** A sensible first value when a rule's field changes. */
function defaultValue(kind: Kind): string {
  return kind === "date" ? "30d" : kind === "bool" ? "true" : kind === "decade" ? "1990" : kind === "choice" ? "albums" : "";
}

/** Rules with a value only (an empty one is still being typed). */
export function completeRules(r: SmartRules): SmartRules {
  return { ...r, rules: r.rules.filter((x) => x.value.trim() !== "") };
}

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/** "123 songs, 7 hr 12 min". */
export function previewLine(p: SmartPreview): string {
  const songs = `${p.count.toLocaleString()} ${p.count === 1 ? "song" : "songs"}`;
  if (!p.count) return "No songs match yet";
  const mins = Math.round(p.durationMs / 60_000);
  const len = mins >= 60 ? `${Math.floor(mins / 60)} hr ${mins % 60} min` : formatDuration(p.durationMs);
  return `${songs}, ${len}`;
}

export function SmartEditorHost() {
  const ed = useCurate((s) => s.editor);
  if (!ed) return null;
  return <SmartPlaylistEditor key={ed.id ?? "new"} initial={ed} />;
}

function RuleRow({ rule, index, onChange, onRemove }: { rule: SmartRule; index: number; onChange: (r: SmartRule) => void; onRemove: () => void }) {
  const kind = fieldKind(rule.field);
  const def = FIELDS.find((f) => f.value === rule.field);
  const n = index + 1;
  let value;
  if (kind === "date") value = <Select label={`Rule ${n} time`} value={rule.value} options={SPANS} onChange={(v) => onChange({ ...rule, value: v })} />;
  else if (kind === "bool")
    value = (
      <Select
        label={`Rule ${n} value`}
        value={rule.value}
        options={[
          { value: "true", label: "Yes" },
          { value: "false", label: "No" },
        ]}
        onChange={(v) => onChange({ ...rule, value: v })}
      />
    );
  else if (kind === "decade") value = <Select label={`Rule ${n} decade`} value={rule.value} options={DECADES} onChange={(v) => onChange({ ...rule, value: v })} />;
  else if (kind === "choice") value = <Select label={`Rule ${n} release type`} value={rule.value} options={RELEASE_TYPES} onChange={(v) => onChange({ ...rule, value: v })} />;
  else
    value = (
      <input
        className="pl-edit__input smart-ed__value"
        aria-label={`Rule ${n} value`}
        value={rule.value}
        inputMode={kind === "number" ? "decimal" : undefined}
        placeholder={def?.hint ?? ""}
        maxLength={200}
        onChange={(e) => onChange({ ...rule, value: e.target.value })}
      />
    );
  return (
    <li className="smart-ed__rule">
      <Select
        label={`Rule ${n} field`}
        value={rule.field}
        options={FIELDS}
        onChange={(f) => {
          const k = fieldKind(f);
          onChange({ field: f, op: OPS[k][0].value, value: k === kind ? rule.value : defaultValue(k) });
        }}
      />
      <Select label={`Rule ${n} comparison`} value={rule.op} options={OPS[kind]} onChange={(op) => onChange({ ...rule, op })} />
      {value}
      <IconButton label={`Remove rule ${n}`} size="sm" icon={<X size={16} />} onClick={onRemove} />
    </li>
  );
}

export function SmartPlaylistEditor({ initial }: { initial: SmartEditorState }) {
  const id = useId();
  const [title, setTitle] = useState(initial.title);
  const [rules, setRules] = useState<SmartRules>(initial.rules);
  const [picks, setPicks] = useState(initial.picks);
  const [promptText, setPromptText] = useState(initial.promptText);
  const [prompting, setPrompting] = useState(false);
  const [preview, setPreview] = useState<SmartPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const editingPlex = !!initial.id && !isPlatterSmart(initial.id);

  const asked = useMemo(() => completeRules(rules), [rules]);
  const pickIds = useMemo(() => picks.map((t) => t.id), [picks]);
  // The count follows the rules (a moment after the last change).
  useEffect(() => {
    let alive = true;
    const t = setTimeout(() => {
      backend.smartPreview(asked, pickIds).then(
        (p) => alive && (setPreview(p), setError(null)),
        (e) => alive && setError(errText(e) || "Check the rules"),
      );
    }, 200);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [asked, pickIds]);

  const update = (i: number, r: SmartRule) => setRules({ ...rules, rules: rules.rules.map((x, j) => (j === i ? r : x)) });
  const remove = (i: number) => setRules({ ...rules, rules: rules.rules.filter((_, j) => j !== i) });
  const add = () => setRules({ ...rules, rules: [...rules.rules, { field: "genre", op: "is", value: "" }] });
  const limitChoice: LimitChoice = rules.limit ? rules.limit.unit : "none";

  const makeRules = async () => {
    if (!promptText.trim()) {
      promptRef.current?.focus();
      return;
    }
    setPrompting(true);
    setError(null);
    try {
      const r = await backend.smartPrompt(promptText);
      setRules(r.rules);
      setPicks(r.picks);
      if (!title.trim()) setTitle(r.title);
    } catch (e) {
      setError(errText(e) || "Couldn't make rules from that");
    }
    setPrompting(false);
  };

  const save = async () => {
    const name = title.trim();
    if (!name) {
      setError("Playlist name is required.");
      nameRef.current?.focus();
      return;
    }
    setSaving(true);
    try {
      const saved = await backend.smartSave({
        id: initial.id,
        title: name,
        rules: asked,
        picks: pickIds,
        prompt: promptText.trim() || null,
      });
      closeSmartEditor();
      if (saved.note) toast.info(saved.note);
      else toast.info(initial.id ? "Smart playlist saved" : `Made "${name}"`);
      nav.go({ name: "playlist", id: saved.id });
    } catch (e) {
      setError(errText(e) || "Couldn't save the smart playlist");
    }
    setSaving(false);
  };

  const heading = initial.id ? "Edit smart playlist" : initial.prompt ? "New prompted playlist" : "New smart playlist";
  const gaps = preview?.plexGaps ?? [];
  const where = editingPlex
    ? "This playlist lives in Plex, so your other Plex apps see the change."
    : initial.id
      ? "A Platter smart playlist, filled on this computer."
      : picks.length
        ? "Picked songs stay first, so this is a Platter smart playlist, filled on this computer."
        : gaps.length
          ? `This will be a Platter smart playlist, filled on this computer: ${gaps.join("; ")}.`
          : "Saved in Plex, so your other Plex apps see it too.";

  return (
    <Dialog open onClose={closeSmartEditor} label={heading} className="pl-edit smart-ed" initialFocus={initial.prompt ? promptRef : nameRef}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <h2 className="pl-edit__title">{heading}</h2>
        {initial.prompt && (
          <div className="smart-ed__prompt">
            <label className="pl-edit__field" htmlFor={`${id}-prompt`}>
              <span className="pl-edit__label">Describe your playlist</span>
              <textarea
                ref={promptRef}
                id={`${id}-prompt`}
                className="pl-edit__input pl-edit__textarea smart-ed__prompt-box"
                value={promptText}
                maxLength={2000}
                placeholder="Mellow jazz from the 60s I haven't played in a while"
                onChange={(e) => setPromptText(e.target.value)}
              />
            </label>
            <div className="smart-ed__prompt-row">
              <p className="smart-ed__note">Your description and your library's genre, style and mood names go to your AI provider, and then the titles of songs that match. Unreleased music is never sent.</p>
              <Button variant="secondary" icon={<Sparkles size={16} />} disabled={prompting} onClick={() => void makeRules()}>
                {prompting ? "Making rules" : "Make rules"}
              </Button>
            </div>
          </div>
        )}
        <label className="pl-edit__field" htmlFor={`${id}-name`}>
          <span className="pl-edit__label">Name</span>
          <input
            ref={nameRef}
            id={`${id}-name`}
            className="pl-edit__input"
            value={title}
            maxLength={100}
            placeholder="My smart playlist"
            onChange={(e) => setTitle(e.target.value)}
          />
        </label>
        <div className="smart-ed__match">
          <span id={`${id}-match`}>Songs that match</span>
          <Select
            labelledBy={`${id}-match`}
            value={rules.matchAll ? "all" : "any"}
            options={[
              { value: "all", label: "all of these rules" },
              { value: "any", label: "any of these rules" },
            ]}
            onChange={(v) => setRules({ ...rules, matchAll: v === "all" })}
          />
        </div>
        <ul className="smart-ed__rules" aria-label="Rules">
          {rules.rules.map((r, i) => (
            <RuleRow key={i} rule={r} index={i} onChange={(x) => update(i, x)} onRemove={() => remove(i)} />
          ))}
        </ul>
        <Button variant="ghost" size="sm" icon={<Plus size={16} />} onClick={add} className="smart-ed__add">
          Add rule
        </Button>
        <div className="smart-ed__options">
          <span className="smart-ed__option">
            <Select
              label="Limit"
              value={limitChoice}
              options={LIMITS}
              onChange={(v) => setRules({ ...rules, limit: v === "none" ? null : { unit: v, value: rules.limit?.value ?? (v === "songs" ? 50 : 2) } })}
            />
            {rules.limit && (
              <input
                className="pl-edit__input smart-ed__num"
                aria-label={rules.limit.unit === "songs" ? "Number of songs" : "Number of hours"}
                inputMode="numeric"
                value={String(rules.limit.value)}
                onChange={(e) => {
                  const n = Math.max(0, Math.min(10_000, Number(e.target.value.replace(/\D/g, "")) || 0));
                  setRules({ ...rules, limit: { unit: rules.limit!.unit, value: n } });
                }}
              />
            )}
          </span>
          <span className="smart-ed__option">
            <span className="smart-ed__option-label" id={`${id}-sort`}>
              Sort
            </span>
            <Select labelledBy={`${id}-sort`} value={rules.sort} options={SORTS} onChange={(v) => setRules({ ...rules, sort: v })} />
          </span>
          <span className="smart-ed__option">
            <span className="smart-ed__option-label" id={`${id}-update`}>
              Update
            </span>
            <Select labelledBy={`${id}-update`} value={rules.update} options={UPDATES} onChange={(v) => setRules({ ...rules, update: v })} />
          </span>
        </div>
        {picks.length > 0 && (
          <section className="smart-ed__picks" aria-label="Picked songs">
            <h3 className="smart-ed__picks-title">Picked for you, first in the playlist</h3>
            <ol>
              {picks.slice(0, 6).map((t) => (
                <li key={t.id} className="smart-ed__pick">
                  <Art path={t.thumb} size={32} />
                  <span className="truncate">
                    {t.title} <span className="smart-ed__pick-artist">{t.trackArtist || t.artistTitle}</span>
                  </span>
                </li>
              ))}
            </ol>
            {picks.length > 6 && <p className="smart-ed__note">and {picks.length - 6} more</p>}
          </section>
        )}
        <p className="smart-ed__count" role="status" aria-live="polite" data-testid="smart-count">
          {preview ? previewLine(preview) : "Counting songs"}
        </p>
        <p className="smart-ed__note">{where}</p>
        {error && (
          <p className="smart-ed__error" role="alert">
            {error}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={closeSmartEditor}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={saving}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
