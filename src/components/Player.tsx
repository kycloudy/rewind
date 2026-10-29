import { useEffect, useRef, useState } from "react";
import { api, fileUrl, isDesktop, on, type Clip } from "../api";
import { clock, dayLabel, duration as fmtDuration, precise, size } from "../format";
import { Icon } from "./Icon";
import { Trimmer } from "./Trimmer";
import type { Toast } from "./Toasts";

type Props = {
  clip: Clip;
  onBack: () => void;
  onChange: (c: Clip) => void;
  onCreated: (c: Clip) => void;
  onDeleted: (id: string) => void;
  onToast: (t: Omit<Toast, "id">) => void;
};

type Job = "trim" | "10" | "50" | null;

// Volume and mute persist across clips and app restarts.
const stored = <T,>(key: string, fallback: T): T => {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
};
const store = (key: string, value: unknown) => {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {}
};

export function Player({ clip, onBack, onChange, onCreated, onDeleted, onToast }: Props) {
  const video = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(() => stored("player.muted", false));
  const [volume, setVolume] = useState(() => stored("player.volume", 1));
  const [time, setTime] = useState(0);
  const [dur, setDur] = useState(clip.duration || 1);
  const [range, setRange] = useState<[number, number]>([0, clip.duration || 1]);
  const [strip, setStrip] = useState<string | null>(null);
  const [title, setTitle] = useState(clip.title);
  const [job, setJob] = useState<Job>(null);
  const [progress, setProgress] = useState(0);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const silent = muted || volume === 0;

  useEffect(() => {
    if (video.current) video.current.volume = volume;
    store("player.volume", volume);
  }, [volume]);

  useEffect(() => store("player.muted", muted), [muted]);

  const changeVolume = (v: number) => {
    setVolume(v);
    if (v > 0) setMuted(false);
  };

  const toggleMute = () => {
    if (volume === 0) changeVolume(1);
    else setMuted((m) => !m);
  };

  const trimmed = range[0] > 0.05 || range[1] < dur - 0.05;

  useEffect(() => {
    api.filmstrip(clip.id).then((p) => setStrip(fileUrl(p)), () => setStrip(null));
    return on<{ id: string; progress: number }>("export-progress", (e) => {
      if (e.id === clip.id) setProgress(e.progress);
    });
  }, [clip.id]);

  const seek = (t: number) => {
    setTime(t);
    if (video.current) video.current.currentTime = t;
  };

  const toggle = () => {
    const v = video.current;
    if (!v) return;
    if (v.paused) {
      if (v.currentTime >= range[1] - 0.05 || v.currentTime < range[0]) v.currentTime = range[0];
      v.play();
    } else v.pause();
  };

  // Keyboard: Space play, I/O set trim points, J/L skip 5 s, M mute, Esc back.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el.closest("input, select, textarea") || (el.getAttribute("role") === "slider" && e.key.startsWith("Arrow"))) return;
      const t = video.current?.currentTime ?? time;
      if (e.key === " ") {
        e.preventDefault();
        toggle();
      } else if (e.key === "i") setRange(([, en]) => [Math.min(t, en - 0.5), en]);
      else if (e.key === "o") setRange(([s]) => [s, Math.max(t, s + 0.5)]);
      else if (e.key === "j") seek(Math.max(0, t - 5));
      else if (e.key === "l") seek(Math.min(dur, t + 5));
      else if (e.key === "m") toggleMute();
      else if (e.key === "Escape") onBack();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const commitTitle = async () => {
    const t = title.trim();
    if (!t || t === clip.title) return setTitle(clip.title);
    try {
      onChange(await api.rename(clip.id, t));
    } catch (e) {
      onToast({ text: String(e), error: true });
      setTitle(clip.title);
    }
  };

  const runExport = async (kind: Exclude<Job, null>) => {
    setJob(kind);
    setProgress(0);
    try {
      const target = kind === "trim" ? null : Number(kind);
      onCreated(await api.exportClip(clip.id, range[0], range[1], target));
    } catch (e) {
      onToast({ text: String(e), error: true });
    } finally {
      setJob(null);
    }
  };

  const act = async (fn: () => Promise<unknown>, done?: string) => {
    try {
      await fn();
      if (done) onToast({ text: done });
    } catch (e) {
      onToast({ text: String(e), error: true });
    }
  };

  const exportButton = (kind: Exclude<Job, null>, label: string, icon: string, disabled = false) => (
    <button className={`btn action${job === kind ? " is-busy" : ""}`} disabled={!!job || disabled} onClick={() => runExport(kind)}>
      <Icon name={icon} size={16} />
      <span>{job === kind ? `Exporting ${Math.round(progress * 100)}%` : label}</span>
      {job === kind && <span className="action-progress" style={{ width: `${progress * 100}%` }} />}
    </button>
  );

  return (
    <div className="player">
      <header className="player-head">
        <button className="btn btn-icon" onClick={onBack} aria-label="Back to clips" title="Back (Esc)">
          <Icon name="back" />
        </button>
        <input
          className="title-input"
          value={title}
          aria-label="Clip name"
          onChange={(e) => setTitle(e.target.value)}
          onBlur={commitTitle}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            if (e.key === "Escape") {
              setTitle(clip.title);
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
        <button
          className={`fav fav-lg${clip.favorite ? " is-on" : ""}`}
          aria-pressed={clip.favorite}
          aria-label={clip.favorite ? "Remove from favorites" : "Add to favorites"}
          onClick={() => act(async () => onChange(await api.favorite(clip.id, !clip.favorite)))}
        >
          <Icon name="star" size={20} fill={clip.favorite} />
        </button>
      </header>

      <div className="player-body">
        <div className="stage-col">
          <div className="stage" onClick={toggle}>
            <video
              ref={video}
              src={isDesktop ? fileUrl(clip.path) : undefined}
              poster={clip.thumb ? fileUrl(clip.thumb) : undefined}
              muted={muted}
              preload="metadata"
              onPlay={() => setPlaying(true)}
              onPause={() => setPlaying(false)}
              onLoadedMetadata={(e) => {
                const d = e.currentTarget.duration;
                if (isFinite(d) && d > 0) {
                  setDur(d);
                  setRange(([s, en]) => [s, Math.min(en, d)]);
                }
              }}
              onTimeUpdate={(e) => {
                const v = e.currentTarget;
                setTime(v.currentTime);
                // Loop inside the trim range.
                if (!v.paused && v.currentTime >= range[1]) v.currentTime = range[0];
              }}
            />
            {!playing && (
              <span className="stage-play" aria-hidden="true">
                <Icon name="play" size={28} />
              </span>
            )}
          </div>

          <div className="transport">
            <button className="btn btn-icon" onClick={toggle} aria-label={playing ? "Pause" : "Play"} title="Play (Space)">
              <Icon name={playing ? "pause" : "play"} />
            </button>
            <span className="transport-time">
              {precise(time)} <span className="dim">/ {fmtDuration(dur)}</span>
            </span>
            <div className="volume">
              <button className="btn btn-icon" onClick={toggleMute} aria-label={silent ? "Unmute" : "Mute"} title="Mute (M)">
                <Icon name={silent ? "mute" : "volume"} />
              </button>
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={muted ? 0 : volume}
                onChange={(e) => changeVolume(Number(e.target.value))}
                aria-label="Volume"
                aria-valuetext={`${Math.round((muted ? 0 : volume) * 100)}%`}
              />
            </div>
          </div>

          <Trimmer
            duration={dur}
            start={range[0]}
            end={range[1]}
            time={time}
            strip={strip}
            onChange={(s, e) => setRange([s, e])}
            onSeek={seek}
          />
        </div>

        <aside className="panel">
          <dl className="facts">
            <div>
              <dt>Game</dt>
              <dd>{clip.game}</dd>
            </div>
            <div>
              <dt>Saved</dt>
              <dd>
                {dayLabel(clip.created)}, {clock(clip.created)}
              </dd>
            </div>
            <div>
              <dt>Length</dt>
              <dd>{fmtDuration(clip.duration)}</dd>
            </div>
            <div>
              <dt>Size</dt>
              <dd>{size(clip.size)}</dd>
            </div>
          </dl>

          <div className="actions">
            {exportButton("trim", "Save trimmed copy", "scissors", !trimmed)}
            {exportButton("10", "Export 10 MB", "send")}
            {exportButton("50", "Export 50 MB", "send")}
          </div>

          <div className="actions">
            <button className="btn action" onClick={() => act(() => api.copy(clip.id), "Copied. Paste it into Discord or a folder.")}>
              <Icon name="copy" size={16} />
              <span>Copy file</span>
            </button>
            <button className="btn action" onClick={() => act(() => api.reveal(clip.id))}>
              <Icon name="folder" size={16} />
              <span>Show in folder</span>
            </button>
            <button
              className={`btn action action-danger${confirmDelete ? " is-armed" : ""}`}
              onClick={() => (confirmDelete ? act(async () => (await api.remove(clip.id), onDeleted(clip.id))) : setConfirmDelete(true))}
              onBlur={() => setConfirmDelete(false)}
            >
              <Icon name="trash" size={16} />
              <span>{confirmDelete ? "Click again to delete" : "Delete"}</span>
            </button>
          </div>
        </aside>
      </div>
    </div>
  );
}
