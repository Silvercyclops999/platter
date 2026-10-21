/**
 * The smart playlist editor, mounted once in the shell: match all or any of
 * a list of rules and groups of rules (each group its own all or any), a
 * limit, a sort and an update schedule, with a live count of the songs the
 * rules find. With an AI provider set, a prompted playlist
 * starts from a description that fills the rules and picks songs.
 */
import { ListPlus, MessageSquareText, Plus, Sparkles, X } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { AppError, Described, SmartField, SmartOp, SmartPreview, SmartRule, SmartRuleGroup, SmartRules, SmartSort, SmartUpdate } from "../lib/types";
import { hasCap } from "../stores/caps";
import { closeSmartEditor, isPlatterSmart, useCurate, type SmartEditorState } from "../stores/curate";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { Art } from "./Art";
import { Button, IconButton } from "./Button";
import { Dialog } from "./Dialog";
import { Select, type SelectOption } from "./Select";
import "./smart-editor.css";

type Kind = "tag" | "text" | "number" | "decade" | "date" | "bool" | "choice" | "list";

export const FIELDS: { value: SmartField; label: string; kind: Kind; hint?: string }[] = [
  { value: "genre", label: "Genre", kind: "tag", hint: "Rock" },
  { value: "style", label: "Style", kind: "tag", hint: "Shoegaze" },
  { value: "mood", label: "Mood", kind: "tag", hint: "Mellow" },
  { value: "title", label: "Title", kind: "text" },
  { value: "artist", label: "Artist", kind: "text" },
  { value: "album", label: "Album", kind: "text" },
  { value: "trackNumber", label: "Track number", kind: "number", hint: "1" },
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
  { value: "era", label: "Era", kind: "text" },
  { value: "played", label: "Played", kind: "date" },
  { value: "skips", label: "Skips", kind: "number", hint: "0" },
  { value: "lastSkipped", label: "Last skipped", kind: "date" },
  { value: "playlist", label: "Playlist", kind: "list" },
  { value: "unreleased", label: "Unreleased", kind: "bool" },
  { value: "lossless", label: "Lossless", kind: "bool" },
  { value: "sampleRate", label: "Sample rate (kHz)", kind: "number", hint: "96" },
  { value: "bitDepth", label: "Bit depth", kind: "number", hint: "24" },
  { value: "finished", label: "Album finished", kind: "bool" },
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
    { value: "beginsWith", label: "begins with" },
    { value: "endsWith", label: "ends with" },
  ],
  number: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
    { value: "gt", label: "is more than" },
    { value: "lt", label: "is less than" },
    { value: "between", label: "is between" },
  ],
  decade: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
  ],
  date: [
    { value: "inLast", label: "in the last" },
    { value: "notInLast", label: "not in the last" },
    { value: "between", label: "between" },
  ],
  bool: [{ value: "is", label: "is" }],
  choice: [
    { value: "is", label: "is" },
    { value: "isNot", label: "is not" },
  ],
  list: [
    { value: "is", label: "has the song" },
    { value: "isNot", label: "doesn't have the song" },
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

/** The core's limits (smart/rules.rs). */
const MAX_RULES = 30;
const MAX_GROUPS = 10;

export const fieldKind = (f: SmartField): Kind => FIELDS.find((x) => x.value === f)?.kind ?? "text";

/** "45d" -> "45 days", "1m" -> "month". */
function spanLabel(v: string): string {
  const m = /^(\d+)(d|w|m|y)$/.exec(v.trim());
  if (!m) return v;
  const n = Number(m[1]);
  const unit = { d: "day", w: "week", m: "month", y: "year" }[m[2] as "d"];
  return n === 1 ? unit : `${n} ${unit}s`;
}

/** A local calendar day as "2026-06-01". */
function isoDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** The last 30 days, as a between value. */
function lastMonthRange(): string {
  const to = new Date();
  const from = new Date(to.getFullYear(), to.getMonth(), to.getDate() - 30);
  return `${isoDay(from)}..${isoDay(to)}`;
}

/** "2026-06-01..2026-08-31" -> "1 Jun 2026 and 31 Aug 2026". */
export function rangeWords(v: string): string {
  const [a, b] = v.split("..");
  const show = (s: string | undefined) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s ?? "");
    if (!m) return s ?? "";
    return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
  };
  return `${show(a)} and ${show(b)}`;
}

const OP_WORDS: Record<SmartOp, string> = {
  is: "is",
  isNot: "is not",
  contains: "contains",
  notContains: "doesn't contain",
  beginsWith: "begins with",
  endsWith: "ends with",
  gt: "over",
  lt: "under",
  inLast: "in the last",
  notInLast: "not in the last",
  between: "between",
};

/** One rule in words: "Mood is Chill", "Length under 3", "Played between 1 Jun 2026 and 31 Aug 2026". */
export function ruleWords(r: SmartRule): string {
  const label = FIELDS.find((f) => f.value === r.field)?.label.replace(/ \((minutes|stars|kHz)\)$/, "") ?? r.field;
  const kind = fieldKind(r.field);
  if (kind === "bool") return `${label}: ${r.value === "true" ? "yes" : "no"}`;
  if (kind === "list") return `${label} "${r.value}" ${r.op === "is" ? "has it" : "doesn't have it"}`;
  let value = r.value;
  if (r.op === "between" && kind === "number") value = r.value.split("..").join(" and ");
  else if (r.op === "between") value = rangeWords(r.value);
  else if (r.op === "inLast" || r.op === "notInLast") value = SPANS.find((s) => s.value === r.value)?.label ?? r.value;
  else if (r.field === "releaseType") value = RELEASE_TYPES.find((t) => t.value === r.value)?.label ?? r.value;
  else if (r.field === "decade") value = `${r.value}s`;
  else if (r.field === "duration") value = `${r.value} min`;
  else if (r.field === "sampleRate") value = `${r.value} kHz`;
  return `${label} ${OP_WORDS[r.op]} ${value}`;
}

/** A sensible first value when a rule's field changes. */
function defaultValue(kind: Kind): string {
  return kind === "date" ? "30d" : kind === "bool" ? "true" : kind === "decade" ? "1990" : kind === "choice" ? "albums" : "";
}

/** A rule still being typed: no value, or one side of a range. */
const unfinished = (x: SmartRule) => x.value.trim() === "" || (x.op === "between" && x.value.split("..").some((side) => side.trim() === ""));

/** Rules with a value only, and groups with such a rule. */
export function completeRules(r: SmartRules): SmartRules {
  const out: SmartRules = { ...r, rules: r.rules.filter((x) => !unfinished(x)) };
  const groups = (r.groups ?? []).map((g) => ({ ...g, rules: g.rules.filter((x) => !unfinished(x)) })).filter((g) => g.rules.length);
  if (groups.length) out.groups = groups;
  else delete out.groups;
  return out;
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

/** Names of your playlists, for rules that name one. */
function usePlaylistNames(on: boolean): string[] {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    backend.playlists().then(
      (all) => alive && setNames([...new Set(all.map((p) => p.title))].sort((a, b) => a.localeCompare(b))),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [on]);
  return names;
}

function RuleRow({ rule, name, listId, onChange, onRemove }: { rule: SmartRule; name: string; listId: string; onChange: (r: SmartRule) => void; onRemove: () => void }) {
  const kind = fieldKind(rule.field);
  const def = FIELDS.find((f) => f.value === rule.field);
  const n = name;
  let value;
  if (kind === "date" && rule.op === "between") {
    const [from = "", to = ""] = rule.value.split("..");
    value = (
      <span className="smart-ed__range">
        <input className="pl-edit__input smart-ed__date" type="date" aria-label={`${n} from`} value={from} onChange={(e) => onChange({ ...rule, value: `${e.target.value}..${to}` })} />
        <span>and</span>
        <input className="pl-edit__input smart-ed__date" type="date" aria-label={`${n} to`} value={to} onChange={(e) => onChange({ ...rule, value: `${from}..${e.target.value}` })} />
      </span>
    );
  } else if (kind === "number" && rule.op === "between") {
    const [from = "", to = ""] = rule.value.split("..");
    const only = (v: string) => v.replace(/[^\d.]/g, "");
    value = (
      <span className="smart-ed__range">
        <input className="pl-edit__input smart-ed__num" inputMode="decimal" aria-label={`${n} from`} value={from} placeholder={def?.hint ?? ""} onChange={(e) => onChange({ ...rule, value: `${only(e.target.value)}..${to}` })} />
        <span>and</span>
        <input className="pl-edit__input smart-ed__num" inputMode="decimal" aria-label={`${n} to`} value={to} onChange={(e) => onChange({ ...rule, value: `${from}..${only(e.target.value)}` })} />
      </span>
    );
  } else if (kind === "date")
  {
    // A span the list doesn't have ("45d" from a description) stays choosable.
    const spans = SPANS.some((s) => s.value === rule.value) ? SPANS : [...SPANS, { value: rule.value, label: spanLabel(rule.value) }];
    value = <Select label={`${n} time`} value={rule.value} options={spans} onChange={(v) => onChange({ ...rule, value: v })} />;
  }
  else if (kind === "bool")
    value = (
      <Select
        label={`${n} value`}
        value={rule.value}
        options={[
          { value: "true", label: "Yes" },
          { value: "false", label: "No" },
        ]}
        onChange={(v) => onChange({ ...rule, value: v })}
      />
    );
  else if (kind === "decade") value = <Select label={`${n} decade`} value={rule.value} options={DECADES} onChange={(v) => onChange({ ...rule, value: v })} />;
  else if (kind === "choice") value = <Select label={`${n} release type`} value={rule.value} options={RELEASE_TYPES} onChange={(v) => onChange({ ...rule, value: v })} />;
  else
    value = (
      <input
        className="pl-edit__input smart-ed__value"
        aria-label={`${n} value`}
        value={rule.value}
        inputMode={kind === "number" ? "decimal" : undefined}
        placeholder={kind === "list" ? "Playlist name" : (def?.hint ?? "")}
        list={kind === "list" ? listId : undefined}
        maxLength={200}
        onChange={(e) => onChange({ ...rule, value: e.target.value })}
      />
    );
  return (
    <li className="smart-ed__rule">
      <Select
        label={`${n} field`}
        value={rule.field}
        options={FIELDS}
        onChange={(f) => {
          const k = fieldKind(f);
          onChange({ field: f, op: OPS[k][0].value, value: k === kind && rule.op !== "between" ? rule.value : defaultValue(k) });
        }}
      />
      <Select
        label={`${n} comparison`}
        value={rule.op}
        options={OPS[kind]}
        onChange={(op) => {
          const was = rule.op === "between";
          let value = rule.value;
          if (kind === "number") {
            // A number range starts from the number there was.
            if (op === "between" && !was) value = `${rule.value}..${rule.value}`;
            else if (op !== "between" && was) value = rule.value.split("..")[0] ?? "";
          } else if (op === "between" && !was) value = lastMonthRange();
          // A range and a span read their values differently.
          else if (op !== "between" && was) value = "30d";
          onChange({ ...rule, op, value });
        }}
      />
      {value}
      <IconButton label={`Remove ${n.toLowerCase()}`} size="sm" icon={<X size={16} />} onClick={onRemove} />
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
  // Two slots: what the live count says about the rules, and what the last
  // action (save, make rules) said. A late count must never clear the second.
  const [ruleError, setRuleError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const describeRef = useRef<HTMLTextAreaElement>(null);
  const editingPlex = !!initial.id && !isPlatterSmart(initial.id);
  const describeAllowed = useSettings((s) => s.settings?.showDescribePlaylist !== false);
  const [describing, setDescribing] = useState(!!initial.describe);
  const [describeText, setDescribeText] = useState("");
  const [described, setDescribed] = useState<Described | null>(null);
  // A name typed by hand stays; otherwise the description names it.
  const named = useRef(!!initial.title);
  // Described playlists stay on this computer unless asked otherwise.
  const [onServer, setOnServer] = useState(false);

  // The description is read as it's typed (on this computer, a moment after the last key).
  useEffect(() => {
    if (!describing) return;
    const text = describeText.trim();
    if (!text) {
      setDescribed(null);
      return;
    }
    let alive = true;
    const t = setTimeout(() => {
      backend.smartDescribe(text).then(
        (d) => {
          if (!alive) return;
          setDescribed(d);
          setRules((r) => ({ ...d.rules, update: r.update }));
          setPicks([]);
          if (!named.current) setTitle(d.title);
        },
        (e) => alive && setError(errText(e) || "Couldn't read that"),
      );
    }, 250);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [describeText, describing]);

  /** Use another reading for a rule the description made. */
  type Offer = Described["choices"][number];
  const ruleAt = (c: Offer) => (c.group == null ? rules.rules[c.index] : rules.groups?.[c.group]?.rules[c.index]);
  const choose = (c: Offer, other: SmartRule) => {
    if (!described) return;
    const old = ruleAt(c);
    if (!old) return;
    const swap = (list: SmartRule[]) => list.map((x, j) => (j === c.index ? other : x));
    if (c.group == null) setRules({ ...rules, rules: swap(rules.rules) });
    else setRules({ ...rules, groups: (rules.groups ?? []).map((g, gi) => (gi === c.group ? { ...g, rules: swap(g.rules) } : g)) });
    setDescribed({
      ...described,
      choices: described.choices.map((x) => (x === c ? { ...x, others: x.others.map((o) => (o === other ? old : o)) } : x)),
    });
  };

  const asked = useMemo(() => completeRules(rules), [rules]);
  const pickIds = useMemo(() => picks.map((t) => t.id), [picks]);
  // The count follows the rules (a moment after the last change).
  useEffect(() => {
    let alive = true;
    const t = setTimeout(() => {
      backend.smartPreview(asked, pickIds).then(
        (p) => alive && (setPreview(p), setRuleError(null)),
        (e) => alive && setRuleError(errText(e) || "Check the rules"),
      );
    }, 200);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [asked, pickIds]);

  const update = (i: number, r: SmartRule) => setRules({ ...rules, rules: rules.rules.map((x, j) => (j === i ? r : x)) });
  const remove = (i: number) => {
    setRules({ ...rules, rules: rules.rules.filter((_, j) => j !== i) });
    // Readings offered for later rules move up with them.
    if (described)
      setDescribed({
        ...described,
        choices: described.choices
          .filter((c) => c.group != null || c.index !== i)
          .map((c) => (c.group == null && c.index > i ? { ...c, index: c.index - 1 } : c)),
      });
  };
  const add = () => setRules({ ...rules, rules: [...rules.rules, { field: "genre", op: "is", value: "" }] });
  // Groups: each its own all or any, one level deep.
  const groups = rules.groups ?? [];
  const setGroups = (next: SmartRuleGroup[]) => setRules({ ...rules, groups: next });
  const setGroup = (g: number, next: SmartRuleGroup | null) =>
    setGroups(next ? groups.map((x, j) => (j === g ? next : x)) : groups.filter((_, j) => j !== g));
  const addGroup = () => setGroups([...groups, { matchAll: false, rules: [{ field: "genre", op: "is", value: "" }] }]);
  const ruleCount = rules.rules.length + groups.reduce((n, g) => n + g.rules.length, 0);
  const full = ruleCount >= MAX_RULES;
  const listId = `${id}-playlists`;
  const playlistNames = usePlaylistNames([...rules.rules, ...groups.flatMap((g) => g.rules)].some((r) => r.field === "playlist"));
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
    setError(null);
    try {
      const saved = await backend.smartSave({
        id: initial.id,
        title: name,
        rules: asked,
        picks: pickIds,
        prompt: (describing ? describeText : promptText).trim() || null,
        // A server that can't keep smart playlists: keep it on this computer.
        // A described one stays here unless the server version was asked for.
        ...(initial.id || (hasCap("serverSmartPlaylists") && !(describing && !onServer)) ? {} : { platter: true }),
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

  const heading = initial.id ? "Edit smart playlist" : initial.prompt ? "New prompted playlist" : describing ? "Describe a playlist" : "New smart playlist";
  const gaps = preview?.plexGaps ?? [];
  // A described playlist can go to the server only when the server can hold its rules.
  const serverOffer = describing && !initial.id && !picks.length && !gaps.length && hasCap("serverSmartPlaylists");
  const where = editingPlex
    ? "This playlist lives in Plex, so your other Plex apps see the change."
    : initial.id
      ? "A Platter smart playlist, filled on this computer."
      : picks.length
        ? "Picked songs stay first, so this is a Platter smart playlist, filled on this computer."
        : gaps.length
          ? `This will be a Platter smart playlist, filled on this computer: ${gaps.join("; ")}.`
          : describing && !(serverOffer && onServer)
            ? "A Platter smart playlist, filled on this computer."
            : "Saved in Plex, so your other Plex apps see it too.";

  return (
    <Dialog open onClose={closeSmartEditor} label={heading} className="pl-edit smart-ed" initialFocus={initial.prompt ? promptRef : initial.describe ? describeRef : nameRef}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <div className="smart-ed__head">
          <h2 className="pl-edit__title">{heading}</h2>
          {describeAllowed && !describing && !initial.prompt && (
            <Button variant="ghost" size="sm" icon={<MessageSquareText size={16} />} onClick={() => (setDescribing(true), setTimeout(() => describeRef.current?.focus(), 0))}>
              Describe a playlist
            </Button>
          )}
        </div>
        {describing && describeAllowed && (
          <div className="smart-ed__prompt smart-ed__describe">
            <label className="pl-edit__field" htmlFor={`${id}-describe`}>
              <span className="pl-edit__label">Describe a playlist</span>
              <textarea
                ref={describeRef}
                id={`${id}-describe`}
                className="pl-edit__input pl-edit__textarea smart-ed__prompt-box"
                value={describeText}
                maxLength={500}
                placeholder="Chill late-night stuff from 2016 I haven't played lately"
                onChange={(e) => setDescribeText(e.target.value)}
              />
            </label>
            {described && described.rules.rules.length + (described.rules.groups?.length ?? 0) + described.unknown.length > 0 && (
              <ul className="smart-ed__chips" aria-label="Understood">
                {rules.rules.map((r, i) => (
                  <li key={i} className="smart-ed__chip">
                    <span>{ruleWords(r)}</span>
                    <button type="button" className="smart-ed__chip-x" aria-label={`Remove ${ruleWords(r)}`} onClick={() => remove(i)}>
                      <X size={12} />
                    </button>
                  </li>
                ))}
                {groups.map((g, gi) => {
                  const words = g.rules.map(ruleWords).join(g.matchAll ? " and " : " or ");
                  return (
                    <li key={`g${gi}`} className="smart-ed__chip">
                      <span>{words}</span>
                      <button
                        type="button"
                        className="smart-ed__chip-x"
                        aria-label={`Remove ${words}`}
                        onClick={() => {
                          setGroup(gi, null);
                          // Offers for the group's words go with it; later groups move up.
                          if (described)
                            setDescribed({
                              ...described,
                              choices: described.choices
                                .filter((c) => c.group !== gi)
                                .map((c) => (c.group != null && c.group > gi ? { ...c, group: c.group - 1 } : c)),
                            });
                        }}
                      >
                        <X size={12} />
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            {described?.choices
              .filter((c) => ruleAt(c))
              .map((c) => (
                <p key={`${c.group ?? ""}-${c.index}-${c.text}`} className="smart-ed__note smart-ed__choice">
                  "{c.text}" could also be{" "}
                  {c.others.map((o, k) => (
                    <button key={k} type="button" className="smart-ed__chip smart-ed__chip--alt" onClick={() => choose(c, o)}>
                      {ruleWords(o)}
                    </button>
                  ))}
                </p>
              ))}
            {!!described?.unknown.length && (
              <p className="smart-ed__note smart-ed__unknown" data-testid="describe-unknown">
                Didn't understand: {described.unknown.join(", ")}
              </p>
            )}
            {described?.notes.map((n) => (
              <p key={n} className="smart-ed__note">
                {n}
              </p>
            ))}
            <p className="smart-ed__note">Read on this computer from your library's own names. Nothing is sent anywhere.</p>
          </div>
        )}
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
            onChange={(e) => {
              named.current = e.target.value.trim() !== "";
              setTitle(e.target.value);
            }}
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
            <RuleRow key={i} rule={r} name={`Rule ${i + 1}`} listId={listId} onChange={(x) => update(i, x)} onRemove={() => remove(i)} />
          ))}
        </ul>
        {groups.map((g, gi) => {
          const label = `Group ${gi + 1}`;
          return (
            <section key={gi} className="smart-ed__group" aria-label={label}>
              <div className="smart-ed__match">
                <Select
                  label={`${label} match`}
                  value={g.matchAll ? "all" : "any"}
                  options={[
                    { value: "any", label: "Any of these" },
                    { value: "all", label: "All of these" },
                  ]}
                  onChange={(v) => setGroup(gi, { ...g, matchAll: v === "all" })}
                />
                <IconButton label={`Remove ${label.toLowerCase()}`} size="sm" icon={<X size={16} />} onClick={() => setGroup(gi, null)} />
              </div>
              <ul className="smart-ed__rules" aria-label={`${label} rules`}>
                {g.rules.map((r, i) => (
                  <RuleRow
                    key={i}
                    rule={r}
                    name={`${label} rule ${i + 1}`}
                    listId={listId}
                    onChange={(x) => setGroup(gi, { ...g, rules: g.rules.map((y, j) => (j === i ? x : y)) })}
                    // The last rule out takes the group with it.
                    onRemove={() => setGroup(gi, g.rules.length > 1 ? { ...g, rules: g.rules.filter((_, j) => j !== i) } : null)}
                  />
                ))}
              </ul>
              <Button
                variant="ghost"
                size="sm"
                icon={<Plus size={16} />}
                disabled={full}
                onClick={() => setGroup(gi, { ...g, rules: [...g.rules, { field: g.rules[g.rules.length - 1]?.field ?? "genre", op: "is", value: "" }] })}
                className="smart-ed__add"
              >
                Add a rule
              </Button>
            </section>
          );
        })}
        <div className="smart-ed__adds">
          <Button variant="ghost" size="sm" icon={<Plus size={16} />} onClick={add} disabled={full} className="smart-ed__add">
            Add rule
          </Button>
          <Button variant="ghost" size="sm" icon={<ListPlus size={16} />} onClick={addGroup} disabled={full || groups.length >= MAX_GROUPS} className="smart-ed__add">
            Add a group
          </Button>
        </div>
        <datalist id={listId}>
          {playlistNames.map((n) => (
            <option key={n} value={n} />
          ))}
        </datalist>
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
        {preview?.notes?.map((n) => (
          <p key={n} className="smart-ed__note smart-ed__warn">
            {n}
          </p>
        ))}
        <p className="smart-ed__note">{where}</p>
        {serverOffer && (
          <label className="smart-ed__server">
            <input type="checkbox" checked={onServer} onChange={(e) => setOnServer(e.target.checked)} />
            Save it on the server instead, so your other apps see it too
          </label>
        )}
        {(error ?? ruleError) && (
          <p className="smart-ed__error" role="alert">
            {error ?? ruleError}
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
