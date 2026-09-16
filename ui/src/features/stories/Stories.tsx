import { useState } from "react";
import { SectionNav, type NavSection } from "../../components/SectionNav";
import { useCharacters, useLocations, useNpcs, useScenes, useStories } from "../../lib/hooks";
import { AssemblySection } from "./AssemblySection";
import { CharacterSheet } from "./CharacterSheet";
import { CharactersSection } from "./CharactersSection";
import { StoryPicker } from "./StoryPicker";
import "./stories.css";
import { TimelineSection } from "./TimelineSection";
import { WorldSection } from "./WorldSection";

type SectionId = "characters" | "world" | "timeline" | "assembly";

const SECTIONS: NavSection[] = [
  { id: "characters", label: "Characters" },
  { id: "world", label: "World" },
  { id: "timeline", label: "Timeline" },
  { id: "assembly", label: "Assembly" },
];

const STORY_ID_KEY = "aiwm:stories:active-story";

/** Story Studio (Phase 1: text + plain image MVP -- see docs/TODO.md "Story
 *  Studio"). A character-consistent illustrated story/comic builder;
 *  Phase 1 deliberately ships without the consistency machinery (IP-Adapter
 *  is Phase 2), so generated portraits will not reliably match across
 *  images yet -- accepted, not a bug. */
export function Stories() {
  const { data: stories, refetch: refetchStories } = useStories();
  const [storyId, setStoryId] = useState<string | null>(() => {
    try {
      return window.localStorage.getItem(STORY_ID_KEY);
    } catch {
      return null;
    }
  });
  const [section, setSection] = useState<SectionId>("timeline");
  const [selectedCharacterId, setSelectedCharacterId] = useState<string | null>(null);
  // Bumped only when a character is opened via the roster's "Edit" control
  // (not on a plain row click) so the sheet's `key` below changes and it
  // remounts straight into edit mode -- see CharacterSheet's `initialEditing`.
  const [editRequestSeq, setEditRequestSeq] = useState(0);
  const [editRequestActive, setEditRequestActive] = useState(false);

  const changeStory = (id: string | null) => {
    setStoryId(id);
    setSelectedCharacterId(null);
    try {
      if (id) window.localStorage.setItem(STORY_ID_KEY, id);
      else window.localStorage.removeItem(STORY_ID_KEY);
    } catch {
      /* localStorage unavailable -- the choice just won't stick across reloads */
    }
  };

  const selectCharacter = (id: string) => {
    setSelectedCharacterId(id);
    setEditRequestActive(false);
  };

  const editCharacter = (id: string) => {
    setSelectedCharacterId(id);
    setEditRequestActive(true);
    setEditRequestSeq((n) => n + 1);
  };

  const story = (stories ?? []).find((s) => s.id === storyId) ?? null;

  const { data: characters, refetch: refetchCharacters } = useCharacters(story?.id ?? null);
  const { data: npcs, refetch: refetchNpcs } = useNpcs(story?.id ?? null);
  const { data: locations, refetch: refetchLocations } = useLocations(story?.id ?? null);
  const { data: scenes, refetch: refetchScenes } = useScenes(story?.id ?? null);

  const refetchAll = () => {
    refetchCharacters();
    refetchNpcs();
    refetchLocations();
    refetchScenes();
  };

  const selectedCharacter = (characters ?? []).find((c) => c.id === selectedCharacterId) ?? null;

  return (
    <div className="stories" data-drawer-open={selectedCharacter != null}>
      <header className="stories__toolbar">
        <StoryPicker stories={stories ?? []} refetch={refetchStories} activeId={storyId} onChange={changeStory} />
      </header>

      {!story ? (
        <div className="card stories__empty">
          <p className="muted">
            Pick a story above, or create one, to start building its cast, world, and timeline.
          </p>
        </div>
      ) : (
        <div className="stories__layout">
          <SectionNav sections={SECTIONS} activeId={section} onChange={(id) => setSection(id as SectionId)} ariaLabel="Story Studio sections" />

          <div className="stories__main">
            {section === "characters" && (
              <CharactersSection
                storyId={story.id}
                characters={characters ?? []}
                selectedId={selectedCharacterId}
                onSelect={selectCharacter}
                onEdit={editCharacter}
                onChanged={refetchCharacters}
              />
            )}
            {section === "world" && (
              <WorldSection story={story} locations={locations ?? []} npcs={npcs ?? []} onChanged={refetchAll} />
            )}
            {section === "timeline" && (
              <TimelineSection
                story={story}
                characters={characters ?? []}
                locations={locations ?? []}
                scenes={scenes ?? []}
                onSelectCharacter={selectCharacter}
                onChanged={refetchAll}
              />
            )}
            {section === "assembly" && <AssemblySection />}
          </div>

          <CharacterSheet
            key={`${selectedCharacterId ?? "none"}:${editRequestSeq}`}
            character={selectedCharacter}
            story={story}
            characters={characters ?? []}
            initialEditing={editRequestActive}
            onClose={() => setSelectedCharacterId(null)}
            onChanged={refetchCharacters}
          />
        </div>
      )}
    </div>
  );
}
