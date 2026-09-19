import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useModelStacks } from "../../lib/hooks";
import type { Captioner, ModelStack } from "../../lib/ipc";
import { TRAINING_TOOLS, stackIdForCaptioner } from "../models/training-tools";
import { useSettling } from "../models/useSettling";
import { useStackInstaller, type StackInstaller } from "../models/useStackInstaller";

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
  const { ids: settling, settle, clear } = useSettling();
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
      settle(s.id);

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
    [refetchCaptioners, onChosen, containerRef, settle],
  );

  const installer = useStackInstaller(watched, onStackInstalled);

  // The registry confirms a settling stack by listing its captioner as
  // installed; its radio can then take focus.
  useEffect(() => {
    for (const c of captioners ?? []) {
      if (!c.installed) continue;
      const stackId = stackIdForCaptioner(c.id);
      if (stackId) clear(stackId);
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
  }, [captioners, containerRef, clear]);

  const startInstall = useCallback(
    (stack: ModelStack) => {
      askedHere.current.add(stack.id);
      void installer.install(stack).then((started) => {
        // A start that failed (offline, repeat click) asked for nothing.
        if (!started) askedHere.current.delete(stack.id);
      });
    },
    [installer],
  );

  return { stacks, installer, settling, notice, startInstall };
}
