import { HelpHint } from "../../components/HelpHint";
import type { GroupModel, LibraryGroup, Need } from "../../lib/ipc";
import { formatGB } from "../../lib/units";
import {
  groupState,
  isWeakSource,
  missingText,
  SOURCE_PHRASE,
  type GroupState,
} from "./packages-view";

type Props = {
  group: LibraryGroup;
  /** Opens the package dialog on this group — its missing parts, or (for a
   *  "works with" group) the optional checkpoint of its own. */
  onComplete: (group: LibraryGroup, why: "missing" | "own-base") => void;
};

/** Icon + word per row status; the word carries the meaning, colour only
 *  repeats it. */
const NEED_STATUS: Record<Need["status"]["kind"], { icon: string; word: string }> = {
  installed: { icon: "✓", word: "Installed" },
  catalog: { icon: "↓", word: "Missing" },
  findable: { icon: "?", word: "Missing" },
  not_runnable: { icon: "✗", word: "Not runnable" },
};

const ROLE_WORD: Record<string, string> = {
  base: "Base",
  vae: "VAE",
  text_encoder: "Text encoder",
};
const roleWord = (role: string) => ROLE_WORD[role] ?? role.replaceAll("_", " ");

/** One base family's part of the library: its runnable state, the base,
 *  the companions and the LoRAs made for it. */
export function PackageGroupCard({ group, onComplete }: Props) {
  const state = groupState(group);
  const g = group;
  return (
    <li className="pkgv-card" data-state={state.kind}>
      <header className="pkgv-card__head">
        <h3 className="pkgv-card__family">{g.family.label}</h3>
        <StateLine state={state} group={g} onComplete={onComplete} />
      </header>
      <ul className="pkgv-rows" aria-label={`${g.family.label}: base and companions`}>
        {g.base ? (
          <InstalledBase base={g.base} />
        ) : (
          g.base_needs.map((n) => <NeedLine key={`${n.role}-${n.label}-${n.optional}`} need={n} />)
        )}
        {g.companions.map((n) => (
          <NeedLine key={`${n.role}-${n.label}`} need={n} />
        ))}
      </ul>
      <LoraList loras={g.loras} family={g.family.label} />
    </li>
  );
}

function StateLine({
  state,
  group,
  onComplete,
}: {
  state: GroupState;
  group: LibraryGroup;
  onComplete: Props["onComplete"];
}) {
  switch (state.kind) {
    case "ready":
      return (
        <p className="pkgv-state" data-state="ready">
          <span aria-hidden="true">✓ </span>
          <strong>Ready</strong> — base and everything it needs are installed.
        </p>
      );
    case "not_runnable":
      return (
        <p className="pkgv-state" data-state="not_runnable">
          <span aria-hidden="true">✗ </span>
          <strong>Not runnable here:</strong> {state.reason}. Its LoRAs will not load; nothing to
          download.
        </p>
      );
    case "works_with":
      return (
        <div className="pkgv-state" data-state="works_with">
          <p>
            <span aria-hidden="true">✓ </span>
            <strong>Works with your {state.baseName}</strong> — made for {group.family.label}.
          </p>
          {state.suggestion && (
            <button type="button" className="pkg__alt" onClick={() => onComplete(group, "own-base")}>
              Get the {group.family.label} checkpoint…
            </button>
          )}
        </div>
      );
    case "missing":
      return (
        <div className="pkgv-state" data-state="missing">
          <p>
            <span aria-hidden="true">↓ </span>
            <strong>Missing:</strong>{" "}
            {state.needs.length > 0 ? missingText(state.needs) : "a base to run on"}
            {state.bytes > 0 && (
              <>
                {" "}— <span className="numeric">{formatGB(state.bytes, 1)}</span>
              </>
            )}
          </p>
          <span className="pkgv-state__action">
            <button type="button" className="pkg__go" onClick={() => onComplete(group, "missing")}>
              Download missing…
            </button>
            <HelpHint area="models" setting="download-missing" />
          </span>
        </div>
      );
  }
}

function InstalledBase({ base }: { base: GroupModel }) {
  return (
    <li className="pkgv-row" data-status="installed">
      <span className="pkgv-row__icon" aria-hidden="true">
        ✓
      </span>
      <span className="pkgv-row__role">Base</span>
      <span className="pkgv-row__name">
        <strong>Installed</strong> — {base.model.name}
      </span>
      <span className="pkgv-row__size numeric">{formatGB(base.model.size_bytes, 1)}</span>
    </li>
  );
}

function NeedLine({ need }: { need: Need }) {
  const s = need.status;
  const { icon, word } = NEED_STATUS[s.kind];
  const size = s.kind === "catalog" ? s.size_bytes : null;
  let detail: string;
  switch (s.kind) {
    case "installed":
      detail = s.made_for ? s.name : `${s.name} (works with it)`;
      break;
    case "catalog":
      detail = `${need.label} — in the catalogue`;
      break;
    case "findable":
      detail = `${need.label} — community checkpoints on Civitai`;
      break;
    case "not_runnable":
      detail = `${need.label} — ${s.reason}`;
      break;
  }
  return (
    <li
      className="pkgv-row"
      data-status={s.kind}
      data-optional={need.optional || undefined}
    >
      <span className="pkgv-row__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="pkgv-row__role">{roleWord(need.role)}</span>
      <span className="pkgv-row__name">
        <strong>{need.optional && s.kind !== "installed" ? "Optional" : word}</strong> — {detail}
      </span>
      <span className="pkgv-row__size numeric">{size != null ? formatGB(size, 1) : ""}</span>
    </li>
  );
}

function LoraList({ loras, family }: { loras: GroupModel[]; family: string }) {
  if (loras.length === 0) {
    return <p className="pkgv-loras pkgv-loras--none">No LoRAs for {family} yet.</p>;
  }
  return (
    <details className="pkgv-loras">
      <summary>
        <span className="numeric">{loras.length}</span> LoRA{loras.length === 1 ? "" : "s"} for{" "}
        {family}
      </summary>
      <ul>
        {loras.map((l) => (
          <li key={l.model.id}>
            <span className="pkgv-loras__name">{l.model.name}</span>
            <span className="pkgv-loras__meta">
              <span className="numeric">{formatGB(l.model.size_bytes, 2)}</span> · base{" "}
              {SOURCE_PHRASE[l.family_source]}
              {isWeakSource(l.family_source) && (
                <strong className="pkgv-weak"> · weak guess</strong>
              )}
            </span>
          </li>
        ))}
      </ul>
    </details>
  );
}
