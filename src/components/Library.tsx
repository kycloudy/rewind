import { useMemo, useState } from "react";
import { fileUrl, type Clip } from "../api";
import { clock, dayLabel, duration, keyLabel } from "../format";
import { Icon } from "./Icon";

export type Filter =
  | { kind: "all" }
  | { kind: "favorites" }
  | { kind: "recordings" }
  | { kind: "exports" }
  | { kind: "game"; game: string };

type Sort = "newest" | "oldest" | "longest";

type Props = {
  clips: Clip[];
  filter: Filter;
  hotkey: string;
  clipSeconds: number;
  onOpen: (id: string) => void;
  onFavorite: (c: Clip) => void;
  onOpenFolder: () => void;
};

const titles: Record<Filter["kind"], string> = {
  all: "All clips",
  favorites: "Favorites",
  recordings: "Recordings",
  exports: "Exports",
  game: "",
};

export function Library({ clips, filter, hotkey, clipSeconds, onOpen, onFavorite, onOpenFolder }: Props) {
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<Sort>("newest");

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = clips.filter((c) => {
      if (filter.kind === "favorites" && !c.favorite) return false;
      if (filter.kind === "recordings" && c.kind !== "recording") return false;
      if (filter.kind === "exports" && c.kind !== "export") return false;
      if (filter.kind === "game" && c.game !== filter.game) return false;
      return !q || c.title.toLowerCase().includes(q) || c.game.toLowerCase().includes(q);
    });
    list.sort((a, b) =>
      sort === "oldest" ? a.created - b.created : sort === "longest" ? b.duration - a.duration : b.created - a.created,
    );
    return list;
  }, [clips, filter, query, sort]);

  // Group by day, except when sorted by length.
  const groups = useMemo(() => {
    if (sort === "longest") return [["", shown] as const];
    const out: [string, Clip[]][] = [];
    for (const c of shown) {
      const label = dayLabel(c.created);
      const last = out[out.length - 1];
      if (last && last[0] === label) last[1].push(c);
      else out.push([label, [c]]);
    }
    return out;
  }, [shown, sort]);

  const heading = filter.kind === "game" ? filter.game : titles[filter.kind];

  return (
    <div className="library">
      <header className="lib-head">
        <h1>{heading}</h1>
        <span className="lib-count">{shown.length}</span>
        <div className="lib-tools">
          <label className="search">
            <Icon name="search" size={16} />
            <input
              type="search"
              placeholder="Search"
              aria-label="Search clips"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          <select aria-label="Sort" value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
            <option value="newest">Newest</option>
            <option value="oldest">Oldest</option>
            <option value="longest">Longest</option>
          </select>
          <button className="btn btn-icon" onClick={onOpenFolder} aria-label="Open clips folder" title="Open clips folder">
            <Icon name="folder" />
          </button>
        </div>
      </header>

      {shown.length === 0 ? (
        <div className="empty">
          {clips.length === 0 ? (
            <>
              <kbd className="empty-key">{keyLabel(hotkey)}</kbd>
              <p>Press {keyLabel(hotkey)} in a game to save the last {clipSeconds} seconds.</p>
            </>
          ) : (
            <p>No clips match.</p>
          )}
        </div>
      ) : (
        <div className="lib-scroll">
          {groups.map(([label, list]) => (
            <section key={label || "all"} className="day">
              {label && <h2 className="day-label">{label}</h2>}
              <div className="grid">
                {list.map((c) => (
                  <ClipTile key={c.id} clip={c} onOpen={() => onOpen(c.id)} onFavorite={() => onFavorite(c)} />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

function ClipTile({ clip, onOpen, onFavorite }: { clip: Clip; onOpen: () => void; onFavorite: () => void }) {
  return (
    <article className="tile">
      <button className="tile-media" onClick={onOpen} aria-label={`Open ${clip.title}`}>
        {clip.thumb ? <img src={fileUrl(clip.thumb)} alt="" loading="lazy" /> : <div className="tile-blank" />}
        <span className="tile-dur">{duration(clip.trim ? clip.trim.end - clip.trim.start : clip.duration)}</span>
        {clip.kind !== "clip" && <span className="tile-kind">{clip.kind === "recording" ? "Recording" : "Export"}</span>}
      </button>
      <div className="tile-meta">
        <div className="tile-text">
          <h3 title={clip.title}>{clip.title}</h3>
          <p>
            {clip.game}, {clock(clip.created)}
          </p>
        </div>
        <button
          className={`fav${clip.favorite ? " is-on" : ""}`}
          onClick={onFavorite}
          aria-pressed={clip.favorite}
          aria-label={clip.favorite ? "Remove from favorites" : "Add to favorites"}
        >
          <Icon name="star" size={16} fill={clip.favorite} />
        </button>
      </div>
    </article>
  );
}
