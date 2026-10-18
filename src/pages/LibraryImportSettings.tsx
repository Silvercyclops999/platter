/**
 * Settings > Library import: the services you can import from, each off
 * until you turn it on, with how to get the file. A source that's on gets
 * Choose file / Choose folder, and once imported, its report line.
 */
import { useState } from "react";
import { useDomId } from "../lib/domId";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import type { ImportReport, ImportSourceId } from "../lib/importTypes";
import { IMPORT_SOURCES, type ImportSourceText } from "../lib/importStrings";
import { backend } from "../lib/backend";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { countParts, importChanged, importFrom, reportLine, useImportOverview } from "./importShared";
import { ImportHistoryLinks } from "./ImportHistoryLinks";
import { PlayedSettings } from "./PlayedALot";
import "./library-import.css";
import { failText } from "../lib/errorText";
import { MoreText } from "./SettingMore";

const SECTION_WORDS = "library import services playlists liked songs history csv m3u transfer played a lot missing plays period";

/** Whether the settings search shows this section. */
export function importMatches(query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${SECTION_WORDS} ${IMPORT_SOURCES.map((s) => `${s.name} ${s.help}`).join(" ")}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

function when(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function SourceRow({ src, on, report }: { src: ImportSourceText; on: boolean | null; report: ImportReport | undefined }) {
  const id = useDomId();
  const [busy, setBusy] = useState(false);
  const toggle = (v: boolean) => {
    const enabled = useSettings.getState().settings?.importSources ?? [];
    const next = v ? [...enabled, src.id] : enabled.filter((x) => x !== src.id);
    void updateSettings({ importSources: next });
  };
  const pick = async (folder: boolean) => {
    setBusy(true);
    await importFrom(src.id as ImportSourceId, folder);
    setBusy(false);
  };
  const forget = () =>
    backend.importForget(src.id).then(
      () => {
        importChanged();
        toast.info(`Removed what was imported from ${src.name}`);
      },
      (e) => toast.error(failText(`Couldn't remove what was imported from ${src.name}`, e)),
    );
  return (
    <div className={`setting-row import-source${on ? " setting-row--wide" : ""}`} data-setting={`import-${src.id}`}>
      <div className="setting-row__text">
        <div className="setting-row__head">
          <div id={`${id}-t`} className="setting-row__title">
            {src.name}
          </div>
          {src.short && <MoreText title={src.name} text={src.help} />}
        </div>
        <p id={`${id}-d`} className="setting-row__desc">
          {src.short ?? src.help}
        </p>
        {src.note && <p className="setting-row__desc import-source__note">{src.note}</p>}
      </div>
      <div className="setting-row__control">
        {on === null ? (
          <Skeleton width={40} height={24} radius="pill" />
        ) : (
          <Switch checked={on} onChange={toggle} labelledBy={`${id}-t`} describedBy={`${id}-d`} />
        )}
      </div>
      {on && (
        <div className="setting-row__block import-source__block">
          <div className="import-source__actions">
            <Button size="sm" onClick={() => void pick(false)} disabled={busy} aria-describedby={`${id}-t`}>
              Choose file
            </Button>
            <Button size="sm" variant="ghost" onClick={() => void pick(true)} disabled={busy} aria-describedby={`${id}-t`}>
              Choose folder
            </Button>
            {busy && (
              <span className="import-source__busy" role="status">
                Importing and matching to your library…
              </span>
            )}
          </div>
          {report && (
            <div className="import-source__report" data-testid={`import-report-${src.id}`}>
              <span className="import-source__found">{reportLine(report)}</span>
              {countParts(report).map((p) => (
                <span key={p} className="import-source__count">
                  {p}
                </span>
              ))}
              <span className="import-source__when">Imported {when(report.importedAt)}</span>
              <ImportHistoryLinks r={report} />
              <span className="import-source__links">
                <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "import" })}>
                  See report
                </Button>
                <Button size="sm" variant="ghost" onClick={() => void forget()}>
                  Remove imported data
                </Button>
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** The Library import section of Settings. */
export function LibraryImportSettings() {
  const s = useSettings((x) => x.settings);
  const [overview] = useImportOverview();
  const enabled = s?.importSources ?? [];
  return (
    <section className="settings__group" aria-labelledby="settings-library-import" data-testid="library-import-settings">
      <h2 id="settings-library-import" className="settings__heading">
        Library import
      </h2>
      <p className="setting-row__desc import-intro">
        Bring your playlists, liked songs, saved albums and listening history from other services and see how much of it your
        library already has. Files are read on this computer. Nothing changes in Plex unless you choose to save a playlist or
        like songs there. Synced.
      </p>
      {IMPORT_SOURCES.map((src) => (
        <SourceRow
          key={src.id}
          src={src}
          on={s ? enabled.includes(src.id) : null}
          report={overview?.reports.find((r) => r.source === src.id)}
        />
      ))}
      <PlayedSettings />
    </section>
  );
}
