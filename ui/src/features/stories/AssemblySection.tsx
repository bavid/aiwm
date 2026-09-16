/** Assembly (final bundling/export -- picking scenes+images to bundle into a
 *  shareable output) is Phase 3. Deliberately just a placeholder: no export
 *  logic belongs here yet. */
export function AssemblySection() {
  return (
    <div className="card">
      <header className="card__head">
        <h2>Assembly</h2>
      </header>
      <p className="muted">
        Bundling a story's scenes into a shareable export (HTML scroll, PDF, CBZ, …) is planned
        for Phase 3 and isn't built yet. Your Timeline is the source of truth in the meantime.
      </p>
    </div>
  );
}
