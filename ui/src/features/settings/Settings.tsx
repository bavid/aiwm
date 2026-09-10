import { useEffect, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useAbout, useRegistryStatus } from "../../lib/hooks";
import {
  getConfig,
  saveConfig,
  setHfToken,
  type AppConfig,
  type AutoPreference,
  type ComfyConfig,
  type ConfigUpdate,
  type LlamaConfig,
  type ModelsConfig,
} from "../../lib/ipc";
import { getTheme, setTheme, type Theme } from "../../lib/theme";
import { BackupCard } from "./BackupCard";
import "./settings.css";

const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(2)} GB`;

const THEME_OPTIONS: { value: Theme; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

const VRAM_MODES: { value: string; label: string }[] = [
  { value: "auto", label: "Auto — let ComfyUI decide from the card" },
  { value: "highvram", label: "High VRAM — keep everything on the GPU" },
  { value: "normalvram", label: "Normal VRAM" },
  { value: "lowvram", label: "Low VRAM — offload aggressively (helps Flux on 16 GB)" },
  { value: "novram", label: "No VRAM — minimal GPU use (slow)" },
];

const AUTO_PREFS: { value: AutoPreference; label: string }[] = [
  { value: "balanced", label: "Balanced — speed, stability and size together" },
  { value: "fast", label: "Prefer fast — highest tokens/sec that still fits" },
  { value: "quality", label: "Prefer quality — biggest model that still fits" },
];

type Form = ConfigUpdate;

const toForm = (c: AppConfig): Form => ({
  store_path: c.store_path,
  offline_mode: c.offline_mode,
  vram_budget_mb: c.vram_budget_mb,
  llama: { ...c.llama },
  comfyui: { ...c.comfyui },
  models: { ...c.models },
});

const sameForm = (a: Form, b: Form): boolean =>
  a.store_path === b.store_path &&
  a.offline_mode === b.offline_mode &&
  a.vram_budget_mb === b.vram_budget_mb &&
  a.llama.gpu_layers === b.llama.gpu_layers &&
  a.llama.ctx_size === b.llama.ctx_size &&
  a.llama.flash_attention === b.llama.flash_attention &&
  a.llama.load_timeout_secs === b.llama.load_timeout_secs &&
  a.llama.jinja === b.llama.jinja &&
  a.llama.chat_template === b.llama.chat_template &&
  a.comfyui.vram_mode === b.comfyui.vram_mode &&
  a.comfyui.reserve_vram_mb === b.comfyui.reserve_vram_mb &&
  a.comfyui.extra_args === b.comfyui.extra_args &&
  a.models.auto_preference === b.models.auto_preference;

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
  const patchComfy = (next: Partial<ComfyConfig>) =>
    patch({ comfyui: { ...form.comfyui, ...next } });
  const patchModels = (next: Partial<ModelsConfig>) =>
    patch({ models: { ...form.models, ...next } });

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
        text: "Saved. Offline mode applies now; store path, VRAM budget, model selection, llama.cpp and ComfyUI options take effect after a restart.",
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
          <h2>Model selection (Auto)</h2>
          <span className="card__sub">restart to apply</span>
        </header>
        <label className="set-field">
          <span>
            When a job asks for <code>Auto</code>, rank the role's models by their
            benchmark (run “Test” on the Models tab) — a model that fits your VRAM
            budget always wins first. No benchmark data → most-recently-used.
          </span>
          <select
            value={form.models.auto_preference}
            onChange={(e) =>
              patchModels({ auto_preference: e.target.value as AutoPreference })
            }
          >
            {AUTO_PREFS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
      </section>

      <HuggingFaceCard />

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
        <label className="set-toggle">
          <input
            type="checkbox"
            checked={form.llama.jinja}
            onChange={(e) => patchLlama({ jinja: e.target.checked })}
          />
          <span>
            <strong>Jinja chat template</strong> — <code>--jinja</code>. Needed for
            tool calls (agents); correct for chat. Turn off only if a model's
            embedded template misbehaves.
          </span>
        </label>
        <label className="set-field">
          <span>
            Chat template override — <code>--chat-template</code>, e.g.{" "}
            <code>qwen2.5-coder</code>. Empty = the GGUF's own.
          </span>
          <input
            type="text"
            value={form.llama.chat_template}
            spellCheck={false}
            onChange={(e) => patchLlama({ chat_template: e.target.value })}
          />
        </label>
      </section>

      <section className="card set-group">
        <header className="card__head">
          <h2>ComfyUI</h2>
          <span className="card__sub">restart to apply</span>
        </header>
        <label className="set-field">
          <span>
            VRAM mode — the <code>--*vram</code> flag ComfyUI starts with
          </span>
          <select
            value={form.comfyui.vram_mode}
            onChange={(e) => patchComfy({ vram_mode: e.target.value })}
          >
            {VRAM_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        <div className="set-grid">
          <label className="set-field set-field--inline">
            <span>
              Reserve VRAM (GB) — <code>--reserve-vram</code>, kept free for the
              OS. <code>0</code> = off. Helps avoid OOM on long video clips.
            </span>
            <input
              type="number"
              min={0}
              max={8}
              step={0.5}
              value={form.comfyui.reserve_vram_mb / 1024}
              onChange={(e) =>
                patchComfy({ reserve_vram_mb: Math.round(numeric(e.target.value, true) * 1024) })
              }
            />
          </label>
        </div>
        <label className="set-field">
          <span>
            Extra args — appended verbatim (power users), e.g.{" "}
            <code>--fast --use-sage-attention</code>
          </span>
          <input
            type="text"
            value={form.comfyui.extra_args}
            spellCheck={false}
            onChange={(e) => patchComfy({ extra_args: e.target.value })}
          />
        </label>
      </section>

      <section className="card set-group">
        <header className="card__head">
          <h2>Generated media</h2>
          <span className="card__sub">read-only</span>
        </header>
        <dl className="set-kv">
          <dt>Folder</dt>
          <dd>{about?.outputs_dir ?? "…"}</dd>
          <dt>Size</dt>
          <dd className="numeric">
            {about ? gb(about.outputs_bytes) : "…"}
            {about?.outputs_dir && (
              <button
                type="button"
                className="set-reveal"
                onClick={() => revealItemInDir(about.outputs_dir).catch(() => {})}
              >
                reveal
              </button>
            )}
          </dd>
        </dl>
        <p className="muted">
          Images and video clips are kept until you delete them — video files are
          large and this folder grows fast. Automatic cleanup / retention limits
          come later; for now, open the folder and prune it yourself.
        </p>
      </section>

      <BackupCard />

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

/** Optional Hugging Face token — for gated repos and higher rate limits. Never
 *  required, stored machine-local (not in a backup), applied on restart. */
function HuggingFaceCard() {
  const { data: status } = useRegistryStatus();
  const [value, setValue] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const save = async (token: string) => {
    setBusy(true);
    setMsg(null);
    try {
      await setHfToken(token);
      setValue("");
      setMsg(
        token.trim()
          ? "Token saved — restart the app to use it."
          : "Token cleared — restart to apply.",
      );
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card set-group">
      <header className="card__head">
        <h2>Hugging Face</h2>
        <span className="card__sub">restart to apply</span>
      </header>
      <label className="set-field">
        <span>
          Access token — optional, only for <strong>gated</strong> repos or if you
          hit the anonymous rate limit. Stored on this machine only (never in a
          backup). Currently: <strong>{status?.token_set ? "set" : "not set"}</strong>.
        </span>
        <input
          type="password"
          value={value}
          placeholder={status?.token_set ? "•••••••• (leave blank to keep)" : "hf_…"}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => {
            setValue(e.target.value);
            setMsg(null);
          }}
        />
      </label>
      <div className="rt-setup">
        <button type="button" disabled={busy || !value.trim()} onClick={() => save(value)}>
          {busy ? "Saving…" : "Save token"}
        </button>
        {status?.token_set && (
          <button
            type="button"
            disabled={busy}
            onClick={() => save("")}
          >
            Clear
          </button>
        )}
        {msg && <span className="muted">{msg}</span>}
      </div>
    </section>
  );
}

/** Parse a number input, treating an empty / bad value as 0. Integer unless
 *  `allowFloat`. */
function numeric(raw: string, allowFloat = false): number {
  const n = Number(raw);
  if (!Number.isFinite(n) || n < 0) return 0;
  return allowFloat ? n : Math.floor(n);
}
