import { useEffect, useState } from "react";
import { useAbout } from "../../lib/hooks";
import {
  getConfig,
  saveConfig,
  type AppConfig,
  type ConfigUpdate,
  type LlamaConfig,
} from "../../lib/ipc";
import { getTheme, setTheme, type Theme } from "../../lib/theme";
import "./settings.css";

const THEME_OPTIONS: { value: Theme; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

type Form = ConfigUpdate;

const toForm = (c: AppConfig): Form => ({
  store_path: c.store_path,
  offline_mode: c.offline_mode,
  vram_budget_mb: c.vram_budget_mb,
  llama: { ...c.llama },
});

const sameForm = (a: Form, b: Form): boolean =>
  a.store_path === b.store_path &&
  a.offline_mode === b.offline_mode &&
  a.vram_budget_mb === b.vram_budget_mb &&
  a.llama.gpu_layers === b.llama.gpu_layers &&
  a.llama.ctx_size === b.llama.ctx_size &&
  a.llama.flash_attention === b.llama.flash_attention &&
  a.llama.load_timeout_secs === b.llama.load_timeout_secs;

export function Settings() {
  const about = useAbout();
  const [loaded, setLoaded] = useState<Form | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [theme, setThemeValue] = useState<Theme>(getTheme());
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ kind: "ok" | "err"; text: string } | null>(null);

  useEffect(() => {
    getConfig()
      .then((c) => {
        const f = toForm(c);
        setLoaded(f);
        setForm(f);
      })
      .catch((e) => setLoadError(e instanceof Error ? e.message : String(e)));
  }, []);

  if (loadError) {
    return (
      <div className="settings">
        <p className="settings__err">Could not load settings: {loadError}</p>
      </div>
    );
  }
  if (!form || !loaded) {
    return (
      <div className="settings">
        <p className="muted">Loading…</p>
      </div>
    );
  }

  const dirty = !sameForm(form, loaded);
  const patch = (next: Partial<Form>) => {
    setForm({ ...form, ...next });
    setStatus(null);
  };
  const patchLlama = (next: Partial<LlamaConfig>) =>
    patch({ llama: { ...form.llama, ...next } });

  const chooseTheme = (t: Theme) => {
    setTheme(t);
    setThemeValue(t);
  };

  const save = async () => {
    setSaving(true);
    setStatus(null);
    try {
      const saved = await saveConfig(form);
      const f = toForm(saved);
      setLoaded(f);
      setForm(f);
      setStatus({
        kind: "ok",
        text: "Saved. Offline mode applies now; store path, VRAM budget and llama.cpp options take effect after a restart.",
      });
    } catch (e) {
      setStatus({ kind: "err", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="settings">
      <header className="settings__head">
        <h1>Settings</h1>
        <p className="muted">
          Stored in <code>config.toml</code>. Most changes need an app restart —
          each field says which.
        </p>
      </header>

      <section className="card set-group">
        <header className="card__head">
          <h2>Appearance</h2>
          <span className="card__sub">applies instantly</span>
        </header>
        <div className="segmented" role="radiogroup" aria-label="Theme">
          {THEME_OPTIONS.map((o) => (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={theme === o.value}
              className="segmented__opt"
              onClick={() => chooseTheme(o.value)}
            >
              {o.label}
            </button>
          ))}
        </div>
      </section>

      <section className="card set-group">
        <header className="card__head">
          <h2>Model store</h2>
          <span className="card__sub">restart to apply</span>
        </header>
        <label className="set-field">
          <span>Canonical directory for imported models</span>
          <input
            type="text"
            value={form.store_path}
            spellCheck={false}
            onChange={(e) => patch({ store_path: e.target.value })}
          />
        </label>
      </section>

      <section className="card set-group">
        <header className="card__head">
          <h2>Scheduler</h2>
          <span className="card__sub">restart to apply</span>
        </header>
        <label className="set-field set-field--inline">
          <span>
            VRAM budget (MB) — <code>0</code> auto-detects the GPU
            {about ? ` · now planning against ${about.vram_budget_mb} MB` : ""}
          </span>
          <input
            type="number"
            min={0}
            step={512}
            value={form.vram_budget_mb}
            onChange={(e) => patch({ vram_budget_mb: numeric(e.target.value) })}
          />
        </label>
      </section>

      <section className="card set-group">
        <header className="card__head">
          <h2>Network</h2>
          <span className="card__sub">applies instantly</span>
        </header>
        <label className="set-toggle">
          <input
            type="checkbox"
            checked={form.offline_mode}
            onChange={(e) => patch({ offline_mode: e.target.checked })}
          />
          <span>
            <strong>Offline mode</strong> — block every outbound network call
            (model downloads, runtime installs).
          </span>
        </label>
      </section>

      <section className="card set-group">
        <header className="card__head">
          <h2>llama.cpp</h2>
          <span className="card__sub">applies on the next model load</span>
        </header>
        <div className="set-grid">
          <label className="set-field set-field--inline">
            <span>
              GPU layers (<code>-ngl</code>) — 999 offloads everything
            </span>
            <input
              type="number"
              min={0}
              value={form.llama.gpu_layers}
              onChange={(e) => patchLlama({ gpu_layers: numeric(e.target.value) })}
            />
          </label>
          <label className="set-field set-field--inline">
            <span>
              Context size (<code>-c</code>) — <code>0</code> caps the model's
              trained context
            </span>
            <input
              type="number"
              min={0}
              step={1024}
              value={form.llama.ctx_size}
              onChange={(e) => patchLlama({ ctx_size: numeric(e.target.value) })}
            />
          </label>
          <label className="set-field set-field--inline">
            <span>Load timeout (seconds)</span>
            <input
              type="number"
              min={10}
              max={3600}
              value={form.llama.load_timeout_secs}
              onChange={(e) => patchLlama({ load_timeout_secs: numeric(e.target.value) })}
            />
          </label>
        </div>
        <label className="set-toggle">
          <input
            type="checkbox"
            checked={form.llama.flash_attention}
            onChange={(e) => patchLlama({ flash_attention: e.target.checked })}
          />
          <span>
            <strong>Flash attention</strong> — <code>--flash-attn on</code>
          </span>
        </label>
      </section>

      <div className="settings__bar">
        {status && (
          <span className={status.kind === "ok" ? "settings__ok" : "settings__err"}>
            {status.text}
          </span>
        )}
        {!status && dirty && <span className="muted">Unsaved changes</span>}
        <button
          type="button"
          className="settings__save"
          disabled={!dirty || saving}
          onClick={save}
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
      </div>
    </div>
  );
}

/** Parse a number input, treating an empty / bad value as 0. */
function numeric(raw: string): number {
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : 0;
}
