import { useCallback, useEffect, useMemo, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { humanize } from "../../lib/errors";
import { useModels } from "../../lib/hooks";
import { libraryPackages, type LibraryGroup, type LibraryPackages } from "../../lib/ipc";
import { OrphanLoras } from "./OrphanLoras";
import { PackageDialog, type PackageSubject } from "./PackageDialog";
import { PackageGroupCard } from "./PackageGroupCard";
import { detectedRows, resolveTarget } from "./packages-view";
import { SaveDetectedDialog } from "./SaveDetectedDialog";
// The buttons and error line share the package dialog's styles.
import "./package-dialog.css";
import "./packages.css";

type Props = {
  onViewDownloads: () => void;
  onAddModels: () => void;
};

/** A cheap fingerprint of the library rows the grouping depends on, so the
 *  view re-reads when a download lands or a family changes elsewhere. */
const libraryKey = (models: { id: string; base_family: string | null }[] | null) =>
  (models ?? []).map((m) => `${m.id}:${m.base_family ?? ""}`).join("|");

/** Models → Packages: the library grouped by base family — base,
 *  companions and the LoRAs made for each — with what is missing, the LoRAs
 *  of unknown base, and the families detected but not yet recorded. Reads
 *  only; every write is a button press. */
export function Packages({ onViewDownloads, onAddModels }: Props) {
  const { data: models } = useModels();
  const [data, setData] = useState<LibraryPackages | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloads, setReloads] = useState(0);
  const [subject, setSubject] = useState<PackageSubject | null>(null);
  const [saving, setSaving] = useState(false);
  const key = libraryKey(models);

  useEffect(() => {
    let alive = true;
    libraryPackages()
      .then((d) => {
        if (!alive) return;
        setData(d);
        setError(null);
      })
      .catch((e) => alive && setError(humanize(e)));
    return () => {
      alive = false;
    };
  }, [key, reloads]);

  const reload = useCallback(() => setReloads((n) => n + 1), []);
  const detected = useMemo(() => (data ? detectedRows(data) : []), [data]);

  // "Download missing" reuses the Get dialog (`PackageDialog`) on a library
  // model rather than queueing the group's catalogue needs directly: the
  // dialog already turns needs into queue requests (catalogue files with
  // pinned hashes, a chosen Civitai checkpoint with its origin), lets the
  // person untick or pick, tags the files as one package for Downloads,
  // and — through `resolvePackage` — fills a findable base with Civitai's
  // checkpoints, which the library grouping leaves empty on purpose.
  const complete = (g: LibraryGroup, why: "missing" | "own-base") => {
    const target = resolveTarget(g);
    if (!target) return;
    setSubject({
      source: "library",
      modelId: target,
      title: g.family.label,
      eyebrow: why === "own-base" ? `Get a checkpoint made for ${g.family.label}` : "Download what is missing",
    });
  };

  const empty = data && data.groups.length === 0 && data.orphans.length === 0;

  return (
    <section className="card card--wide pkgv" aria-labelledby="pkgv-title">
      <header className="card__head">
        <h2 id="pkgv-title">
          Packages <HelpHint area="models" setting="packages" />
        </h2>
        <span className="card__sub numeric">
          {data ? `${data.groups.length} base famil${data.groups.length === 1 ? "y" : "ies"}` : ""}
        </span>
        {detected.length > 0 && (
          <span className="pkgv-save">
            <button type="button" className="pkg__alt" onClick={() => setSaving(true)}>
              Save detected families (<span className="numeric">{detected.length}</span>)
            </button>
            <HelpHint area="models" setting="save-detected" />
          </span>
        )}
      </header>
      <p className="muted pkgv-intro">
        Your library by base model: a LoRA only renders on the base it was made for, with that
        base's text encoders and VAE.
      </p>
      {error && (
        <p className="pkg__error" role="alert">
          Could not read the library: {error}
        </p>
      )}
      {!data && !error && <p className="muted">Grouping the library…</p>}
      {empty && (
        <div className="pkgv-empty">
          <p>
            No image or video models yet. Get a stack under Add models, or search Civitai in
            Discover — Get there brings the base along with a LoRA.
          </p>
          <button type="button" className="pkg__go" onClick={onAddModels}>
            Go to Add models
          </button>
        </div>
      )}
      {data && data.groups.length > 0 && (
        <ul className="pkgv-grid" aria-label="Base families">
          {data.groups.map((g) => (
            <PackageGroupCard key={g.family.id} group={g} onComplete={complete} />
          ))}
        </ul>
      )}
      {data && <OrphanLoras orphans={data.orphans} onChanged={reload} />}
      {subject && (
        <PackageDialog
          subject={subject}
          onClose={() => setSubject(null)}
          onViewDownloads={() => {
            setSubject(null);
            onViewDownloads();
          }}
        />
      )}
      {saving && (
        <SaveDetectedDialog rows={detected} onClose={() => setSaving(false)} onSaved={reload} />
      )}
    </section>
  );
}
