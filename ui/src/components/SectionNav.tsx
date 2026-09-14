import "./section-nav.css";

export interface NavSection {
  id: string;
  label: string;
}

/** Left-docked page nav for a tab whose content used to be one long scroll
 *  (Settings, Models) -- pick a section instead of scrolling past everything
 *  else to find it. */
export function SectionNav({
  sections,
  activeId,
  onChange,
  ariaLabel,
}: {
  sections: NavSection[];
  activeId: string;
  onChange: (id: string) => void;
  ariaLabel: string;
}) {
  return (
    <nav className="section-nav" aria-label={ariaLabel}>
      {sections.map((s) => (
        <button
          key={s.id}
          type="button"
          className={s.id === activeId ? "section-nav__item section-nav__item--active" : "section-nav__item"}
          aria-current={s.id === activeId}
          onClick={() => onChange(s.id)}
        >
          {s.label}
        </button>
      ))}
    </nav>
  );
}
