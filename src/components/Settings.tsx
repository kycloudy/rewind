import { useEffect, useState, type ReactNode } from "react";
import { fetchReleases, type Release } from "../releases";
import { api, appVersion, openUrl, pickFolder, type Devices, type Settings, type UpdateInfo } from "../api";
import { keyLabel } from "../format";
import { Markdown } from "./Markdown";

type Props = {
  settings: Settings;
  onSaved: (s: Settings) => void;
  onError: (text: string) => void;
  update: UpdateInfo | null;
  updating: number | null;
  onCheckUpdate: () => Promise<void>;
  onInstallUpdate: () => void;
};

const REPO_URL = "https://github.com/kycloudy/rewind";

const CODEC_LABELS: Record<Settings["codec"], string> = { h264: "H.264", hevc: "HEVC", av1: "AV1" };
const CODEC_ORDER: Settings["codec"][] = ["h264", "hevc", "av1"];

export function SettingsView({ settings, onSaved, onError, update, updating, onCheckUpdate, onInstallUpdate }: Props) {
  const [draft, setDraft] = useState(settings);
  const [devices, setDevices] = useState<Devices | null>(null);
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);
  const [releases, setReleases] = useState<Release[] | null>(null);
  const [version, setVersion] = useState("");
  const [showNotes, setShowNotes] = useState(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);

  useEffect(() => setDraft(settings), [settings]);
  useEffect(() => {
    api.devices().then(setDevices);
    appVersion().then(setVersion);
    fetchReleases().then(setReleases, () => setReleases([]));
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

  const encoders = devices?.encoders ?? [];
  const gpus = encoders.filter((e) => e.codec === "h264");
  // Codecs the chosen GPU can encode. H.264 always works.
  const codecs = (vendor?: string) =>
    CODEC_ORDER.filter((c) => c === "h264" || encoders.some((e) => e.codec === c && e.vendor === vendor));
  const chosenVendor = gpus.find((g) => g.name === draft.encoder)?.vendor ?? gpus[0]?.vendor;

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
          {draft.gamesOnly && (
            <Row label="Other games">
              <input
                type="text"
                value={draft.gameApps.join(", ")}
                placeholder="game.exe, other.exe"
                onChange={(e) => set("gameApps", e.target.value.split(",").map((g) => g.trim()).filter(Boolean))}
              />
            </Row>
          )}
          {draft.gamesOnly && (
            <Row label="Not games">
              <input
                type="text"
                value={draft.notGames.join(", ")}
                placeholder="app.exe, other.exe"
                onChange={(e) => set("notGames", e.target.value.split(",").map((g) => g.trim()).filter(Boolean))}
              />
            </Row>
          )}
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
            <select
              value={draft.encoder}
              onChange={(e) => {
                const encoder = e.target.value;
                set("encoder", encoder);
                const vendor = gpus.find((g) => g.name === encoder)?.vendor ?? gpus[0]?.vendor;
                if (!codecs(vendor).includes(draft.codec)) set("codec", "h264");
              }}
            >
              <option value="auto">Automatic{gpus[0] ? ` (${gpus[0].label})` : ""}</option>
              {gpus.map((e) => (
                <option key={e.name} value={e.name}>
                  {e.label}
                </option>
              ))}
            </select>
          </Row>
          <Row label="Codec">
            <select value={draft.codec} onChange={(e) => set("codec", e.target.value as Settings["codec"])}>
              {codecs(chosenVendor).map((c) => (
                <option key={c} value={c}>
                  {CODEC_LABELS[c]}
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
              <Row label="Noise suppression">
                <Switch checked={draft.micDenoise} onChange={(v) => set("micDenoise", v)} label="Noise suppression" />
              </Row>
              <Row label="Noise gate">
                <Switch checked={draft.micGate} onChange={(v) => set("micGate", v)} label="Noise gate" />
              </Row>
              {draft.micGate && (
                <Row label="Gate threshold">
                  <div className="slider-row">
                    <input
                      type="range"
                      min={-80}
                      max={-10}
                      step={1}
                      value={draft.micGateThreshold}
                      onChange={(e) => set("micGateThreshold", Number(e.target.value))}
                      aria-label="Gate threshold"
                    />
                    <span className="slider-val">{draft.micGateThreshold} dB</span>
                  </div>
                </Row>
              )}
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

        <Group title="About">
          <Row label="Rewind">
            <span className="muted">{version}</span>
          </Row>
          <Row label="GitHub">
            <button className="btn" onClick={() => openUrl(REPO_URL)}>
              kycloudy/rewind
            </button>
          </Row>
          <Row label="Updates">
            <div className="about-actions">
              <button className="btn" onClick={() => setShowNotes((v) => !v)} aria-expanded={showNotes}>
                What's new
              </button>
              {updating !== null ? (
                <button className="btn" disabled>
                  {updating < 1 ? `Downloading ${Math.round(updating * 100)}%` : "Installing"}
                </button>
              ) : update?.available ? (
                <button className="btn btn-primary" onClick={onInstallUpdate}>
                  Update to {update.available}
                </button>
              ) : (
                <button
                  className="btn"
                  disabled={checking}
                  onClick={async () => {
                    setChecking(true);
                    await onCheckUpdate();
                    setChecking(false);
                  }}
                >
                  {checking ? "Checking" : "Check for updates"}
                </button>
              )}
            </div>
          </Row>
          {showNotes &&
            (releases ?? []).map((r) => (
              <article className="release" key={r.tag}>
                <header>
                  <h3>{r.name}</h3>
                  <time>{new Date(r.date).toLocaleDateString()}</time>
                </header>
                <Markdown source={r.body} />
              </article>
            ))}
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
