import { useEffect, useRef, useState } from "react";
import type { Settings, Status } from "../api";
import type { Filter } from "./Library";
import { duration, keyLabel } from "../format";
import { Icon } from "./Icon";

type Props = {
  status: Status | null;
  settings: Settings | null;
  games: [string, number][];
  filter: Filter | null;
  settingsOpen: boolean;
  onFilter: (f: Filter) => void;
  onSettings: () => void;
  onSave: () => void;
  onRecord: () => void;
};

const TICKS = 15;

/** Re-renders every `ms` while `active`. */
function useNow(active: boolean, ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [active, ms]);
  return now;
}

export function Sidebar(p: Props) {
  const st = p.status;
  const buffering = !!st?.buffering;
  const since = useRef<number | null>(null);
  if (buffering && since.current === null) since.current = Date.now();
  if (!buffering) since.current = null;

  const now = useNow(buffering || !!st?.recording, 500);
  const seconds = st?.clipSeconds ?? 30;
  const filled = buffering && since.current ? Math.min(1, (now - since.current) / 1000 / seconds) : 0;
  const lit = Math.round(filled * TICKS);

  const isActive = (f: Filter) =>
    p.filter && p.filter.kind === f.kind && (f.kind !== "game" || (p.filter.kind === "game" && p.filter.game === f.game));

  const nav = (f: Filter, label: string, icon: string, count?: number) => (
    <button className={`nav-item${isActive(f) ? " is-active" : ""}`} onClick={() => p.onFilter(f)}>
      <Icon name={icon} size={16} />
      <span className="nav-label">{label}</span>
      {count !== undefined && <span className="nav-count">{count}</span>}
    </button>
  );

  return (
    <aside className="sidebar">
      <div className="brand">
        <img src="/icon.png" alt="" width={22} height={22} />
        <span>Rewind</span>
      </div>

      <section className={`deck${buffering ? " is-live" : ""}${st?.recording ? " is-recording" : ""}`} aria-label="Capture">
        <div className="deck-head">
          <span className="deck-dot" />
          <span className="deck-state">
            {st?.recording
              ? "Recording"
              : buffering
                ? "Replay on"
                : st?.waitingForGame
                  ? "Waiting for a game"
                  : st?.error
                    ? "Capture stopped"
                    : "Replay off"}
          </span>
          {st?.recording && st.recordingSince && (
            <span className="deck-timer">{duration((now - st.recordingSince) / 1000)}</span>
          )}
        </div>
        <div className="tape" aria-hidden="true">
          {Array.from({ length: TICKS }, (_, i) => (
            <span key={i} className={i < lit ? "on" : ""} />
          ))}
        </div>
        {st?.error && <p className="deck-error">{st.error}</p>}
        <button className="btn btn-primary deck-save" disabled={!buffering || st?.saving} onClick={p.onSave}>
          <span>{st?.saving ? "Saving" : `Save ${seconds} s`}</span>
          {p.settings && <kbd>{keyLabel(p.settings.clipHotkey)}</kbd>}
        </button>
        <button
          className={`btn deck-record${st?.recording ? " is-on" : ""}`}
          disabled={!buffering && !st?.recording && !st?.waitingForGame}
          onClick={p.onRecord}
        >
          <span className="rec-glyph" />
          <span>{st?.recording ? "Stop" : "Record"}</span>
          {p.settings && <kbd>{keyLabel(p.settings.recordHotkey)}</kbd>}
        </button>
        {st?.encoder && <p className="deck-encoder">{st.encoder}</p>}
      </section>

      <nav className="nav">
        {nav({ kind: "all" }, "All clips", "grid")}
        {nav({ kind: "favorites" }, "Favorites", "star")}
        {nav({ kind: "recordings" }, "Recordings", "film")}
        {nav({ kind: "exports" }, "Exports", "export")}
      </nav>

      {p.games.length > 0 && (
        <nav className="nav nav-games" aria-label="Games">
          <h2 className="nav-heading">Games</h2>
          {p.games.map(([g, n]) => (
            <button
              key={g}
              className={`nav-item${isActive({ kind: "game", game: g }) ? " is-active" : ""}`}
              onClick={() => p.onFilter({ kind: "game", game: g })}
            >
              <span className="game-mark" style={{ background: hue(g) }}>
                {g.slice(0, 1)}
              </span>
              <span className="nav-label">{g}</span>
              <span className="nav-count">{n}</span>
            </button>
          ))}
        </nav>
      )}

      <button className={`nav-item nav-settings${p.settingsOpen ? " is-active" : ""}`} onClick={p.onSettings}>
        <Icon name="gear" size={16} />
        <span className="nav-label">Settings</span>
      </button>
    </aside>
  );
}

/** Stable muted color per game name. */
export function hue(name: string): string {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return `oklch(0.42 0.09 ${h})`;
}
