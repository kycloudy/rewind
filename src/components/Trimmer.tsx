import { useRef } from "react";
import { precise } from "../format";

type Props = {
  duration: number;
  start: number;
  end: number;
  time: number;
  strip: string | null;
  onChange: (start: number, end: number) => void;
  onSeek: (t: number) => void;
};

const MIN = 0.5;

export function Trimmer({ duration, start, end, time, strip, onChange, onSeek }: Props) {
  const track = useRef<HTMLDivElement>(null);
  const pct = (t: number) => `${(t / duration) * 100}%`;

  const timeAt = (clientX: number) => {
    const r = track.current!.getBoundingClientRect();
    return Math.min(duration, Math.max(0, ((clientX - r.left) / r.width) * duration));
  };

  const drag = (which: "start" | "end") => (e: React.PointerEvent) => {
    e.stopPropagation();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const t = timeAt(ev.clientX);
      if (which === "start") {
        const s = Math.min(t, end - MIN);
        onChange(s, end);
        onSeek(s);
      } else {
        const en = Math.max(t, start + MIN);
        onChange(start, en);
        onSeek(en);
      }
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const nudge = (which: "start" | "end") => (e: React.KeyboardEvent) => {
    const step = e.shiftKey ? 1 : 0.1;
    const d = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
    if (!d) return;
    e.preventDefault();
    if (which === "start") onChange(Math.max(0, Math.min(start + d, end - MIN)), end);
    else onChange(start, Math.min(duration, Math.max(end + d, start + MIN)));
  };

  return (
    <div className="trimmer">
      <div className="trim-track" ref={track} onPointerDown={(e) => onSeek(timeAt(e.clientX))}>
        <div className="trim-strip" style={strip ? { backgroundImage: `url("${strip}")` } : undefined} />
        <div className="trim-shade" style={{ left: 0, width: pct(start) }} />
        <div className="trim-shade" style={{ left: pct(end), right: 0 }} />
        <div className="trim-window" style={{ left: pct(start), width: pct(end - start) }} />
        <button
          className="trim-handle is-start"
          style={{ left: pct(start) }}
          onPointerDown={drag("start")}
          onKeyDown={nudge("start")}
          role="slider"
          aria-label="Trim start"
          aria-valuemin={0}
          aria-valuemax={duration}
          aria-valuenow={start}
          aria-valuetext={precise(start)}
        />
        <button
          className="trim-handle is-end"
          style={{ left: pct(end) }}
          onPointerDown={drag("end")}
          onKeyDown={nudge("end")}
          role="slider"
          aria-label="Trim end"
          aria-valuemin={0}
          aria-valuemax={duration}
          aria-valuenow={end}
          aria-valuetext={precise(end)}
        />
        <div className="trim-playhead" style={{ left: pct(time) }} />
      </div>
      <div className="trim-readout">
        <span>{precise(start)}</span>
        <span className="trim-len">{(end - start).toFixed(1)} s</span>
        <span>{precise(end)}</span>
      </div>
    </div>
  );
}
