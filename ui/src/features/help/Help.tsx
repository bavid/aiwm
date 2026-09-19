import { useEffect, useId, useMemo, useState, type ReactNode } from "react";
import { SectionNav, type NavSection } from "../../components/SectionNav";
import {
  HELP_AREAS,
  findSetting,
  findTopic,
  searchHelp,
  settingDomId,
  topicDomId,
  type HelpArea,
  type HelpBlock,
  type HelpSetting,
  type HelpTopic,
} from "../../help/index.ts";
import type { HelpFocus } from "../../help/HelpContext.ts";
import "./help.css";

/** How long the jumped-to target keeps its highlight. */
const FLASH_MS = 1600;

const SECTIONS: NavSection[] = HELP_AREAS.map((a) => ({ id: a.id, label: a.label }));

/** `text` with every case-insensitive occurrence of `query` wrapped in
 *  `<mark>`; the plain text when the query is empty. */
function Highlight({ text, query }: { text: string; query: string }) {
  const q = query.trim().toLowerCase();
  if (!q) return <>{text}</>;
  const parts: ReactNode[] = [];
  const lower = text.toLowerCase();
  let from = 0;
  for (;;) {
    const at = lower.indexOf(q, from);
    if (at < 0) break;
    if (at > from) parts.push(text.slice(from, at));
    parts.push(<mark key={at}>{text.slice(at, at + q.length)}</mark>);
    from = at + q.length;
  }
  if (from < text.length) parts.push(text.slice(from));
  return <>{parts}</>;
}

function Block({ block, query }: { block: HelpBlock; query: string }) {
  if (block.kind === "p") {
    return (
      <p>
        <Highlight text={block.text} query={query} />
      </p>
    );
  }
  const items = block.items.map((item, i) => (
    <li key={i}>
      <Highlight text={item} query={query} />
    </li>
  ));
  return block.kind === "steps" ? <ol>{items}</ol> : <ul>{items}</ul>;
}

function SettingEntry({
  area,
  setting,
  query,
  isFlashing,
}: {
  area: HelpArea;
  setting: HelpSetting;
  query: string;
  isFlashing: boolean;
}) {
  return (
    <article
      id={settingDomId(area, setting.key)}
      className="help__setting"
      data-flash={isFlashing || undefined}
    >
      <h4 tabIndex={-1}>
        <Highlight text={setting.label} query={query} />
      </h4>
      <dl className="help__qa">
        <dt>What does it do?</dt>
        <dd>
          <Highlight text={setting.what} query={query} />
        </dd>
        <dt>Why do I need it?</dt>
        <dd>
          <Highlight text={setting.why} query={query} />
        </dd>
        <dt>What happens when I change or start it?</dt>
        <dd>
          <Highlight text={setting.effect} query={query} />
        </dd>
        <dt>What do I gain?</dt>
        <dd>
          <Highlight text={setting.benefit} query={query} />
        </dd>
        {setting.pitfalls && (
          <>
            <dt>Watch out</dt>
            <dd>
              <Highlight text={setting.pitfalls} query={query} />
            </dd>
          </>
        )}
        {setting.measured && (
          <>
            <dt>Measured</dt>
            <dd className="numeric">
              <Highlight text={setting.measured} query={query} />
            </dd>
          </>
        )}
      </dl>
    </article>
  );
}

function TopicSection({
  topic,
  query,
  settings,
  showBody,
  flashId,
}: {
  topic: HelpTopic;
  query: string;
  /** Which of the topic's settings to render (all, or the search hits). */
  settings: readonly HelpSetting[];
  showBody: boolean;
  /** DOM id of the element to highlight, if it is in this topic. */
  flashId: string | null;
}) {
  const id = topicDomId(topic);
  return (
    <section id={id} className="card help__topic" data-flash={flashId === id || undefined}>
      <h3 tabIndex={-1}>
        <Highlight text={topic.title} query={query} />
      </h3>
      <p className="help__summary">
        <Highlight text={topic.summary} query={query} />
      </p>
      {showBody && topic.body.map((b, i) => <Block key={i} block={b} query={query} />)}
      {settings.length > 0 && (
        <div className="help__settings">
          {settings.map((s) => (
            <SettingEntry
              key={s.key}
              area={topic.area}
              setting={s}
              query={query}
              isFlashing={flashId === settingDomId(topic.area, s.key)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

type Props = {
  /** A jump request from a hint, the palette or another tab; `null` otherwise. */
  focus: HelpFocus | null;
  /** Clears the request so re-visiting the tab does not jump again. */
  onFocusConsumed: () => void;
};

/** The Help tab: one section per area, a search box over everything, and
 *  deep links from the `?` hints. All content is in the app; nothing is
 *  fetched. */
export function Help({ focus, onFocusConsumed }: Props) {
  const [section, setSection] = useState<HelpArea>(HELP_AREAS[0].id);
  const [query, setQuery] = useState("");
  /** DOM id of the element a jump landed on, while it is highlighted. */
  const [flashId, setFlashId] = useState<string | null>(null);
  const searchId = useId();

  // A jump: resolve the target (a setting key first, then a topic id, else
  // the area itself), clear any search so the target is rendered, and hand
  // the id to the effect below, which runs once the section has committed.
  useEffect(() => {
    if (!focus) return;
    const hit = focus.key ? findSetting(focus.area, focus.key) : null;
    const topic = hit?.topic ?? (focus.key ? findTopic(focus.area, focus.key) : null);
    const id = hit
      ? settingDomId(focus.area, focus.key ?? "")
      : topic
        ? topicDomId(topic)
        : null;
    setSection(focus.area);
    setQuery("");
    setFlashId(id);
    onFocusConsumed();
  }, [focus, onFocusConsumed]);

  // An instant scroll, not a smooth one: the reader arrives from another tab
  // and has not seen this page yet, so there is nothing to animate from —
  // and a smooth scroll never finishes while the window is not painting.
  useEffect(() => {
    if (!flashId) return;
    const el = document.getElementById(flashId);
    el?.scrollIntoView({ block: "center" });
    el?.querySelector<HTMLElement>("h3, h4")?.focus({ preventScroll: true });
    const timer = window.setTimeout(() => setFlashId(null), FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [flashId]);

  const isSearching = query.trim() !== "";
  const results = useMemo(() => searchHelp(query), [query]);
  const area = HELP_AREAS.find((a) => a.id === section) ?? HELP_AREAS[0];

  const status = !isSearching
    ? ""
    : results.count === 0
      ? "No matches"
      : `${results.count} ${results.count === 1 ? "match" : "matches"}`;

  return (
    <div className="help">
      <header className="help__head">
        <h2>Help</h2>
        <div className="help__search">
          <label htmlFor={searchId} className="visually-hidden">
            Search the help
          </label>
          <input
            id={searchId}
            type="search"
            value={query}
            placeholder="Search settings and topics…"
            onChange={(e) => setQuery(e.target.value)}
          />
          {isSearching && (
            <button type="button" className="chip" onClick={() => setQuery("")}>
              Clear
            </button>
          )}
        </div>
        <p className="help__status muted" role="status">
          {status}
        </p>
      </header>

      <div className="help__body">
        <SectionNav
          sections={SECTIONS}
          activeId={section}
          onChange={(id) => {
            setSection(id as HelpArea);
            setQuery("");
          }}
          ariaLabel="Help sections"
        />
        <div className="help__sections">
          {isSearching ? (
            results.hits.length === 0 ? (
              <p className="card help__empty">
                Nothing in the help mentions “{query.trim()}”. Try a shorter word.
              </p>
            ) : (
              results.hits.map((hit) => (
                <TopicSection
                  key={`${hit.topic.area}-${hit.topic.id}`}
                  topic={hit.topic}
                  query={query}
                  settings={hit.settings}
                  showBody={hit.topicMatched}
                  flashId={null}
                />
              ))
            )
          ) : (
            area.topics.map((topic) => (
              <TopicSection
                key={topic.id}
                topic={topic}
                query=""
                settings={topic.settings ?? []}
                showBody
                flashId={flashId}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}
