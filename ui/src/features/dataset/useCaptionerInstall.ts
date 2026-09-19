import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useModelStacks } from "../../lib/hooks";
import type { Captioner, ModelStack } from "../../lib/ipc";
import { TRAINING_TOOLS } from "../models/training-tools";
import { useStackInstaller, type StackInstaller } from "../models/useStackInstaller";

/** How long a just-finished stack may wait for the captioner registry to
 *  confirm it before a "not usable" verdict is shown (one registry poll plus
 *  slack). */
const SETTLE_MS = 8000;

type Options = {
  captioners: readonly Captioner[] | null;
  refetchCaptioners: () => void;
  /** Select this captioner and switch auto-captioning on. */
  onChosen: (captionerId: string) => void;
  /** The captioning fieldset — focus moves to the new radio only when the
   *  person was working inside it. */
  containerRef: RefObject<HTMLElement | null>;
};

export interface CaptionerInstall {
  /** The catalog's captioner stacks (`null` while loading). */
  stacks: readonly ModelStack[] | null;
  installer: StackInstaller;
  /** Stack ids whose files just finished and whose captioner the registry
   *  has not confirmed yet. */
  settling: ReadonlySet<string>;
  /** The completion message for the status region ("" until one). */
  notice: string;
  /** Install a stack from this form. */
  startInstall: (stack: ModelStack) => void;
}

function withoutId(set: ReadonlySet<string>, id: string): ReadonlySet<string> {
  return new Set([...set].filter((x) => x !== id));
}

/** The Dataset form's captioner installs. A finished install always
 *  refreshes the registry; it only *changes the form* (selects the captioner,
 *  turns auto-captioning on, announces it, moves focus to its radio) when
 *  the Install was clicked here, or when no captioner was installed before —
 *  an install started on the Models tab never rewrites a choice made here. */
export function useCaptionerInstall({
  captioners,
  refetchCaptioners,
  onChosen,
  containerRef,
}: Options): CaptionerInstall {
  const allStacks = useModelStacks();
  const stacks = useMemo(
    () => (allStacks ? allStacks.filter((s) => TRAINING_TOOLS[s.id]?.captionerId) : null),
    [allStacks],
  );
  const watched = useMemo(() => stacks ?? [], [stacks]);

  const askedHere = useRef<Set<string>>(new Set());
  const installedCount = useRef(0);
  const focusRadio = useRef<string | null>(null);
  const [settling, setSettling] = useState<ReadonlySet<string>>(new Set());
  const [notice, setNotice] = useState("");

  useEffect(() => {
    installedCount.current = (captioners ?? []).filter((c) => c.installed).length;
  }, [captioners]);

  const onStackInstalled = useCallback(
    (s: ModelStack) => {
      refetchCaptioners();
      const tool = TRAINING_TOOLS[s.id];
      const captionerId = tool?.captionerId;
      if (!captionerId) return;
      setSettling((prev) => new Set(prev).add(s.id));
      setTimeout(() => setSettling((prev) => withoutId(prev, s.id)), SETTLE_MS);

      const mine = askedHere.current.delete(s.id);
      if (!mine && installedCount.current > 0) return;
      onChosen(captionerId);
      if (!mine) return;
      setNotice(`${tool.shortName} installed — it is selected for auto-captioning.`);
      const active = document.activeElement;
      const workingHere =
        active === null || active === document.body || !!containerRef.current?.contains(active);
      focusRadio.current = workingHere ? captionerId : null;
    },
    [refetchCaptioners, onChosen, containerRef],
  );

  const installer = useStackInstaller(watched, onStackInstalled);

  // The registry confirms a settling stack by listing its captioner as
  // installed; its radio can then take focus.
  useEffect(() => {
    for (const c of captioners ?? []) {
      if (!c.installed) continue;
      const stackId = Object.keys(TRAINING_TOOLS).find((k) => TRAINING_TOOLS[k].captionerId === c.id);
      if (stackId) setSettling((prev) => (prev.has(stackId) ? withoutId(prev, stackId) : prev));
    }
    const want = focusRadio.current;
    if (!want) return;
    const radio = containerRef.current?.querySelector<HTMLInputElement>(
      `input[type="radio"][value="${CSS.escape(want)}"]`,
    );
    if (!radio) return;
    focusRadio.current = null;
    const active = document.activeElement;
    if (active === null || active === document.body || containerRef.current?.contains(active)) {
      radio.focus();
    }
  }, [captioners, containerRef]);

  const startInstall = useCallback(
    (stack: ModelStack) => {
      askedHere.current.add(stack.id);
      void installer.install(stack);
    },
    [installer],
  );

  return { stacks, installer, settling, notice, startInstall };
}
