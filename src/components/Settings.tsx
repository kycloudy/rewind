import { useEffect, useState, type ReactNode } from "react";
import { api, pickFolder, type Devices, type Settings } from "../api";
import { keyLabel } from "../format";

type Props = {
  settings: Settings;
  onSaved: (s: Settings) => void;
  onError: (text: string) => void;
};

export function SettingsView({ settings, onSaved, onError }: Props) {
  const [draft, setDraft] = useState(settings);
  const [devices, setDevices] = useState<Devices | null>(null);
  const [saving, setSaving] = useState(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);

  useEffect(() => setDraft(settings), [settings]);
  useEffect(() => {
    api.devices().then(setDevices);
  }, []);

  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => setDraft((d) => ({ ...d, [k]: v }));

  const save = async () => {
    setSaving(true);
    try {
      onSaved(await api.saveSettings(draft));
    } catch (e) {
      onError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="settings">
      <header className="lib-head">
        <h1>Settings</h1>
      </header>
      <div className="settings-scroll">
        <Group title="Replay">
          <Row label="Replay buffer">
            <Switch checked={draft.bufferEnabled} onChange={(v) => set("bufferEnabled", v)} label="Replay buffer" />
          </Row>
          <Row label="Only in games">
            <Switch checked={draft.gamesOnly} onChange={(v) => set("gamesOnly", v)} label="Only in games" />
          </Row>
          <Row label="Clip length">
            <Segmented
              value={String(draft.clipSeconds)}
              options={[
                ["15", "15 s"],
                ["30", "30 s"],
                ["60", "1 min"],
                ["120", "2 min"],
                ["300", "5 min"],
              ]}
              onChange={(v) => set("clipSeconds", Number(v))}
            />
          </Row>
        </Group>

        <Group title="Video">
          <Row label="Quality">
            <Segmented
              value={draft.quality}
              options={[
                ["low", "Low"],
                ["medium", "Medium"],
                ["high", "High"],
                ["ultra", "Ultra"],
              ]}
              onChange={(v) => set("quality", v as Settings["quality"])}
            />
          </Row>
          <Row label="Resolution">
            <Segmented
              value={draft.resolution}
              options={[
                ["native", "Native"],
                ["1440", "1440p"],
                ["1080", "1080p"],
                ["720", "720p"],
              ]}
              onChange={(v) => set("resolution", v as Settings["resolution"])}
            />
          </Row>
          <Row label="Frame rate">
            <Segmented
              value={String(draft.fps)}
              options={[
                ["30", "30 fps"],
                ["60", "60 fps"],
              ]}
              onChange={(v) => set("fps", Number(v))}
            />
          </Row>
          <Row label="Display">
            <select value={draft.monitor} onChange={(e) => set("monitor", Number(e.target.value))}>
              {(devices?.monitors ?? [{ index: draft.monitor, name: "Main display", width: 0, height: 0, primary: true }]).map((m) => (
                <option key={m.index} value={m.index}>
                  {m.name}
                  {m.width ? ` (${m.width} × ${m.height})` : ""}
                </option>
              ))}
            </select>
          </Row>
          <Row label="Encoder">
            <select value={draft.encoder} onChange={(e) => set("encoder", e.target.value)}>
              <option value="auto">Automatic{devices?.encoders[0] ? ` (${devices.encoders[0].label})` : ""}</option>
              {devices?.encoders.map((e) => (
                <option key={e.name} value={e.name}>
                  {e.label}
                </option>
              ))}
            </select>
          </Row>
        </Group>

        <Group title="Audio">
          <Row label="Game and system sound">
            <Switch checked={draft.systemAudio} onChange={(v) => set("systemAudio", v)} label="Game and system sound" />
          </Row>
          <Row label="Microphone">
            <Switch checked={draft.mic} onChange={(v) => set("mic", v)} label="Microphone" />
          </Row>
          {draft.mic && (
            <>
              <Row label="Microphone device">
                <select value={draft.micDevice ?? ""} onChange={(e) => set("micDevice", e.target.value || null)}>
                  <option value="">Windows default</option>
                  {devices?.mics.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              </Row>
              <Row label="Microphone volume">
                <div className="slider-row">
                  <input
                    type="range"
                    min={0}
                    max={2}
                    step={0.05}
                    value={draft.micVolume}
                    onChange={(e) => set("micVolume", Number(e.target.value))}
                    aria-label="Microphone volume"
                  />
                  <span className="slider-val">{Math.round(draft.micVolume * 100)}%</span>
                </div>
              </Row>
            </>
          )}
        </Group>

        <Group title="Hotkeys">
          <Row label="Save clip">
            <HotkeyInput value={draft.clipHotkey} onChange={(v) => set("clipHotkey", v)} />
          </Row>
          <Row label="Start or stop recording">
            <HotkeyInput value={draft.recordHotkey} onChange={(v) => set("recordHotkey", v)} />
          </Row>
          <Row label="Play a sound">
            <Switch checked={draft.clipSound} onChange={(v) => set("clipSound", v)} label="Play a sound" />
          </Row>
        </Group>

        <Group title="Storage and startup">
          <Row label="Clips folder">
            <div className="folder-row">
              <span className="folder-path" title={draft.clipsDir}>
                {draft.clipsDir}
              </span>
              <button
                className="btn"
                onClick={async () => {
                  const f = await pickFolder();
                  if (f) set("clipsDir", f);
                }}
              >
                Change
              </button>
            </div>
          </Row>
          <Row label="Start with Windows">
            <Switch checked={draft.launchAtLogin} onChange={(v) => set("launchAtLogin", v)} label="Start with Windows" />
          </Row>
        </Group>
      </div>

      <footer className={`savebar${dirty ? " is-shown" : ""}`} aria-hidden={!dirty}>
        <button className="btn" onClick={() => setDraft(settings)} tabIndex={dirty ? 0 : -1}>
          Discard
        </button>
        <button className="btn btn-primary" onClick={save} disabled={saving} tabIndex={dirty ? 0 : -1}>
          {saving ? "Saving" : "Save changes"}
        </button>
      </footer>
    </div>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="group">
      <h2>{title}</h2>
      <div className="group-rows">{children}</div>
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="row">
      <span className="row-label">{label}</span>
      <div className="row-control">{children}</div>
    </div>
  );
}

function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button role="switch" aria-checked={checked} aria-label={label} className="switch" onClick={() => onChange(!checked)}>
      <span />
    </button>
  );
}

function Segmented({ value, options, onChange }: { value: string; options: [string, string][]; onChange: (v: string) => void }) {
  return (
    <div className="seg" role="radiogroup">
      {options.map(([v, label]) => (
        <button key={v} role="radio" aria-checked={value === v} className={value === v ? "is-on" : ""} onClick={() => onChange(v)}>
          {label}
        </button>
      ))}
    </div>
  );
}

const MODS = ["Control", "Alt", "Shift", "Meta"];

/** Records a key press as a Tauri shortcut string, e.g. "Alt+KeyS" or "F8". */
function HotkeyInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [listening, setListening] = useState(false);
  return (
    <button
      className={`hotkey${listening ? " is-listening" : ""}`}
      onClick={() => setListening(true)}
      onBlur={() => setListening(false)}
      onKeyDown={(e) => {
        if (!listening) return;
        e.preventDefault();
        if (e.key === "Escape") return setListening(false);
        if (MODS.includes(e.key)) return;
        const parts = [];
        if (e.ctrlKey) parts.push("Control");
        if (e.altKey) parts.push("Alt");
        if (e.shiftKey) parts.push("Shift");
        if (e.metaKey) parts.push("Super");
        parts.push(e.code);
        onChange(parts.join("+"));
        setListening(false);
      }}
    >
      {listening ? "Press a key" : <kbd>{keyLabel(value)}</kbd>}
    </button>
  );
}
