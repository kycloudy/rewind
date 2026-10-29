import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, on, type Clip, type Settings, type Status, type UpdateInfo } from "./api";
import { Sidebar } from "./components/Sidebar";
import { Library, type Filter } from "./components/Library";
import { Player } from "./components/Player";
import { SettingsView } from "./components/Settings";
import { Toasts, type Toast } from "./components/Toasts";

type View = { kind: "library" } | { kind: "clip"; id: string } | { kind: "settings" };

export function App() {
  const [clips, setClips] = useState<Clip[]>([]);
  const [status, setStatus] = useState<Status | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [view, setView] = useState<View>({ kind: "library" });
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [appUpdate, setAppUpdate] = useState<UpdateInfo | null>(null);
  /** Download progress from 0 to 1 while an update installs. */
  const [updating, setUpdating] = useState<number | null>(null);

  const toast = useCallback((t: Omit<Toast, "id">) => {
    const id = Math.random().toString(36).slice(2);
    setToasts((ts) => [...ts.slice(-2), { ...t, id }]);
    setTimeout(() => setToasts((ts) => ts.filter((x) => x.id !== id)), t.action ? 6000 : 4000);
  }, []);

  const refresh = useCallback(() => api.clips().then(setClips), []);

  const installUpdate = useCallback(async () => {
    setUpdating(0);
    try {
      await api.installUpdate();
    } catch (e) {
      toast({ text: String(e), error: true });
      setUpdating(null);
    }
  }, [toast]);

  const checkUpdate = useCallback(async () => {
    try {
      const u = await api.checkUpdate();
      setAppUpdate(u);
      if (!u.available) toast({ text: "Rewind is up to date" });
    } catch (e) {
      toast({ text: String(e), error: true });
    }
  }, [toast]);

  useEffect(() => {
    refresh();
    api.status().then(setStatus);
    api.settings().then(setSettings);
    const offs = [
      on<Status>("status", setStatus),
      on<Clip>("clip-saved", (c) => {
        setClips((cs) => [c, ...cs.filter((x) => x.id !== c.id)]);
        toast({
          text: c.kind === "recording" ? "Recording saved" : "Clip saved",
          action: { label: "Open", run: () => setView({ kind: "clip", id: c.id }) },
        });
      }),
      on<string>("notice", (text) => toast({ text, error: true })),
      on<number>("update-progress", setUpdating),
      on<Settings>("settings", setSettings),
    ];
    return () => offs.forEach((f) => f());
  }, [refresh, toast]);

  useEffect(() => {
    let live = true;
    api
      .checkUpdate()
      .then((u) => {
        if (!live) return;
        setAppUpdate(u);
        if (u.available) toast({ text: `Rewind ${u.available} is available`, action: { label: "Update", run: installUpdate } });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [toast, installUpdate]);
  // The first time a full-screen guess starts capture, offer to undo it.
  const offered = useRef(new Set<string>());
  const guess = status?.game?.guessed && settings?.gamesOnly ? status.game : null;
  useEffect(() => {
    if (!guess || offered.current.has(guess.exe)) return;
    offered.current.add(guess.exe);
    toast({
      text: `Capturing ${guess.name}`,
      action: { label: "Not a game", run: () => runAction(async () => setSettings(await api.notAGame(guess.exe))) },
    });
  }, [guess?.exe]);

  const update = useCallback((c: Clip) => setClips((cs) => cs.map((x) => (x.id === c.id ? c : x))), []);
  const removed = useCallback((id: string) => setClips((cs) => cs.filter((x) => x.id !== id)), []);

  const current = view.kind === "clip" ? clips.find((c) => c.id === view.id) : undefined;
  const games = useMemo(() => {
    const m = new Map<string, number>();
    clips.forEach((c) => m.set(c.game, (m.get(c.game) ?? 0) + 1));
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [clips]);

  const runAction = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
    } catch (e) {
      toast({ text: String(e), error: true });
    }
  };

  return (
    <div className="app">
      <Sidebar
        status={status}
        settings={settings}
        games={games}
        filter={view.kind === "library" ? filter : null}
        settingsOpen={view.kind === "settings"}
        onFilter={(f) => {
          setFilter(f);
          setView({ kind: "library" });
        }}
        onSettings={() => setView({ kind: "settings" })}
        onSave={() => runAction(api.saveClip)}
        onRecord={() => runAction(api.toggleRecording)}
      />
      <main className="main">
        {view.kind === "settings" && settings ? (
          <SettingsView
            settings={settings}
            seen={status?.seen ?? null}
            onSaved={(s) => {
              setSettings(s);
              toast({ text: "Settings saved" });
            }}
            onError={(text) => toast({ text, error: true })}
            update={appUpdate}
            updating={updating}
            onCheckUpdate={checkUpdate}
            onInstallUpdate={installUpdate}
          />
        ) : current ? (
          <Player
            key={current.id}
            clip={current}
            onBack={() => setView({ kind: "library" })}
            onChange={update}
            onCreated={(c) => {
              setClips((cs) => [c, ...cs]);
              toast({ text: "Export saved", action: { label: "Open", run: () => setView({ kind: "clip", id: c.id }) } });
            }}
            onDeleted={(id) => {
              removed(id);
              setView({ kind: "library" });
              toast({ text: "Clip deleted" });
            }}
            onToast={toast}
          />
        ) : (
          <Library
            clips={clips}
            filter={filter}
            hotkey={settings?.clipHotkey ?? "F8"}
            clipSeconds={settings?.clipSeconds ?? 30}
            onOpen={(id) => setView({ kind: "clip", id })}
            onFavorite={async (c) => update(await api.favorite(c.id, !c.favorite))}
            onOpenFolder={() => api.openFolder()}
          />
        )}
      </main>
      <Toasts toasts={toasts} onDismiss={(id) => setToasts((ts) => ts.filter((t) => t.id !== id))} />
    </div>
  );
}
