export interface SettingsSection {
  id: string;
  label: string;
}

/** Left-docked section nav for the Settings page -- same "pick a page,
 *  don't scroll a mile" idea as the Chat tab's session sidebar, just for a
 *  fixed list of sections instead of a dynamic one. */
export function SettingsNav({
  sections,
  activeId,
  onChange,
}: {
  sections: SettingsSection[];
  activeId: string;
  onChange: (id: string) => void;
}) {
  return (
    <nav className="settings-nav" aria-label="Settings sections">
      {sections.map((s) => (
        <button
          key={s.id}
          type="button"
          className={
            s.id === activeId ? "settings-nav__item settings-nav__item--active" : "settings-nav__item"
          }
          aria-current={s.id === activeId}
          onClick={() => onChange(s.id)}
        >
          {s.label}
        </button>
      ))}
    </nav>
  );
}
