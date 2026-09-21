import { useId } from "react";
import type { CheckpointCandidate, KnownModel, Need } from "../../lib/ipc";
import { formatGB } from "../../lib/units";
import type { NeedChoice } from "./package-plan";

const ROLE_LABEL: Record<string, string> = {
  base: "Base model",
  vae: "VAE",
  text_encoder: "Text encoder",
};

const roleLabel = (role: string) => ROLE_LABEL[role] ?? role.replaceAll("_", " ");

const count = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : `${n}`;

/** Icon + word for a need's status — the word carries the meaning, the icon
 *  and colour only repeat it. */
const STATUS: Record<Need["status"]["kind"], { icon: string; word: string }> = {
  installed: { icon: "✓", word: "Installed" },
  catalog: { icon: "↓", word: "Download" },
  findable: { icon: "?", word: "Choose a checkpoint" },
  not_runnable: { icon: "✗", word: "Not runnable" },
};

type Props = {
  need: Need;
  choice: NeedChoice;
  known: readonly KnownModel[] | null;
  disabled: boolean;
  onChange: (next: NeedChoice) => void;
};

/** One need of a package: its role, what it is, and where it stands. A
 *  catalog file shows its size; a findable base offers Civitai's top
 *  checkpoints to pick from; an optional suggestion has its own tick box and
 *  starts unticked. */
export function PackageNeedRow({ need, choice, known, disabled, onChange }: Props) {
  const s = need.status;
  const { icon, word } = STATUS[s.kind];
  const includeId = useId();
  const fetchable = s.kind === "catalog" || s.kind === "findable";
  const size =
    s.kind === "catalog"
      ? s.size_bytes
      : s.kind === "findable"
        ? (s.candidates[choice.candidate]?.file?.size ?? null)
        : null;

  return (
    <li
      className="pkg__need"
      data-status={s.kind}
      data-optional={need.optional || undefined}
      data-skipped={(fetchable && !choice.include) || undefined}
    >
      <span className="pkg__icon" aria-hidden="true">
        {icon}
      </span>
      <div className="pkg__needmain">
        <div className="pkg__needhead">
          <span className="pkg__role">{roleLabel(need.role)}</span>
          <span className="pkg__label">{need.label}</span>
          {need.optional && <span className="badge">optional</span>}
        </div>
        <p className="pkg__status">
          <strong>{word}</strong>
          {s.kind === "installed" &&
            (s.made_for ? ` — ${s.name}` : ` — works with your ${s.name} — made for ${need.label}`)}
          {s.kind === "catalog" &&
            ` — from the catalogue${s.name !== need.label ? `: ${s.name}` : ""}`}
          {s.kind === "not_runnable" && ` — ${s.reason}`}
          {s.kind === "catalog" && known && !known.some((k) => k.id === s.known_model_id) && (
            <span className="pkg__warn"> — not in the catalogue this app loaded; use Add models</span>
          )}
        </p>
        {fetchable && need.optional && (
          <span className="chip pkg__include">
            <input
              id={includeId}
              type="checkbox"
              checked={choice.include}
              disabled={disabled}
              onChange={(e) => onChange({ ...choice, include: e.target.checked })}
            />
            <label htmlFor={includeId}>Also download this (optional)</label>
          </span>
        )}
        {s.kind === "findable" && (
          <CandidateList
            label={need.label}
            baseLabel={s.base_label}
            candidates={s.candidates}
            note={s.note}
            choice={choice}
            disabled={disabled}
            onChange={onChange}
          />
        )}
      </div>
      <span className="pkg__size numeric">{size != null ? formatGB(size, 1) : ""}</span>
    </li>
  );
}

function CandidateList({
  label,
  baseLabel,
  candidates,
  note,
  choice,
  disabled,
  onChange,
}: {
  label: string;
  baseLabel: string;
  candidates: readonly CheckpointCandidate[];
  note: string | null;
  choice: NeedChoice;
  disabled: boolean;
  onChange: (next: NeedChoice) => void;
}) {
  const name = useId();
  if (candidates.length === 0) {
    return (
      <p className="pkg__note">
        {note ?? `Civitai lists no checkpoints for “${baseLabel}” right now.`}
      </p>
    );
  }
  return (
    <fieldset className="pkg__candidates" disabled={disabled}>
      <legend className="visually-hidden">Which {label} to download</legend>
      {candidates.map((c, i) => {
        const id = `${name}-${i}`;
        const noFile = !c.file?.download_url;
        return (
          <div key={`${c.model_id}/${c.version_id}`} className="pkg__candidate">
            <input
              id={id}
              type="radio"
              name={name}
              checked={choice.candidate === i}
              disabled={noFile}
              // Picking a checkpoint means wanting it: tick the optional box too.
              onChange={() => onChange({ include: true, candidate: i })}
              onClick={() => onChange({ include: true, candidate: i })}
            />
            <label htmlFor={id}>
              <span className="pkg__cname">{c.name}</span>
              <span className="pkg__cmeta numeric">
                ↓ {count(c.downloads)}
                {c.file ? ` · ${formatGB(c.file.size, 1)}` : ""}
                {noFile && " · no downloadable file"}
              </span>
            </label>
          </div>
        );
      })}
    </fieldset>
  );
}
