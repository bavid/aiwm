import { useState } from "react";
import type { PermissionDecision } from "../../lib/ipc";
import {
  approvalDomId,
  clockOf,
  DECISION_LABEL,
  permissionAction,
  toolBlurb,
  TOOL_STATUS_LABEL,
} from "./labels";
import type { Block, PermissionBlock, ToolBlock } from "./transcript";

/** How many lines of a tool's output show before it is folded. A `cat` of a
 *  long file used to push everything else off the screen. */
const PREVIEW_LINES = 12;

/** One transcript block — an agent paragraph, a tool-call card, an inline
 *  approval prompt, an end-of-turn marker, or an error. */
export function TranscriptBlock({
  block,
  decision,
  onAnswer,
}: {
  block: Block;
  decision: PermissionDecision | null;
  onAnswer: (requestId: string, decision: PermissionDecision) => void;
}) {
  if (block.block === "text") {
    return (
      <div className="tblock tblock--text">
        <span className="tblock__who">
          Agent
          {block.ts && <span className="tblock__time numeric"> {clockOf(block.ts)}</span>}
        </span>
        <p className="tblock__body">{block.text}</p>
      </div>
    );
  }
  if (block.block === "idle") {
    return (
      <p className="tblock tblock--idle">
        <span aria-hidden="true">— </span>turn finished, your move<span aria-hidden="true"> —</span>
      </p>
    );
  }
  if (block.block === "error") {
    return (
      <div className="tblock tblock--error">
        <span className="tblock__who">Runtime error</span>
        <p className="tblock__body">{block.message}</p>
      </div>
    );
  }
  if (block.block === "tool") return <ToolCard block={block} />;
  return <Approval block={block} decision={decision} onAnswer={onAnswer} />;
}

function ToolCard({ block }: { block: ToolBlock }) {
  const [expanded, setExpanded] = useState(false);
  const lines = block.output ? block.output.split("\n") : [];
  const foldable = lines.length > PREVIEW_LINES;
  const shown = foldable && !expanded ? lines.slice(0, PREVIEW_LINES).join("\n") : block.output;
  const gloss = toolBlurb(block.name);

  return (
    <div className="tool" data-status={block.status}>
      <div className="tool__head">
        <span className="tool__name">{block.name}</span>
        {gloss && <span className="tool__gloss">{gloss}</span>}
        <span className="tool__status" data-status={block.status}>
          {TOOL_STATUS_LABEL[block.status]}
        </span>
        {block.ts && <span className="tool__time numeric">{clockOf(block.ts)}</span>}
      </div>
      {block.command && <pre className="tool__cmd">{block.command}</pre>}
      {block.output && (
        <>
          <pre className="tool__out" data-folded={foldable && !expanded}>
            {shown}
          </pre>
          {foldable && (
            <button
              type="button"
              className="tool__more"
              aria-expanded={expanded}
              onClick={() => setExpanded((v) => !v)}
            >
              {expanded
                ? `Fold ${lines.length} lines of output`
                : `Show all ${lines.length} lines of output`}
            </button>
          )}
        </>
      )}
    </div>
  );
}

function Approval({
  block,
  decision,
  onAnswer,
}: {
  block: PermissionBlock;
  decision: PermissionDecision | null;
  onAnswer: (requestId: string, decision: PermissionDecision) => void;
}) {
  const answered = decision != null;
  return (
    <section
      className="approval"
      id={approvalDomId(block.id)}
      data-answered={answered}
      aria-label={`Approval: ${permissionAction(block.kind)}`}
      tabIndex={-1}
    >
      <header className="approval__head">
        <span className="badge badge--warn">{answered ? "answered" : "approval needed"}</span>
        <span className="approval__kind">
          The agent wants to {permissionAction(block.kind)}.
        </span>
      </header>
      <pre className="approval__cmd">{block.summary}</pre>
      {answered ? (
        <p className="approval__outcome">
          <span aria-hidden="true">{decision === "deny" ? "✕" : "✓"}</span>{" "}
          {DECISION_LABEL[decision]}
        </p>
      ) : (
        <div className="approval__actions">
          <button
            type="button"
            className="approval__allow"
            onClick={() => onAnswer(block.id, "allow_once")}
          >
            Allow once
          </button>
          {block.always && (
            <button type="button" onClick={() => onAnswer(block.id, "allow_always")}>
              Allow <code className="approval__pattern">{block.always}</code> all session
            </button>
          )}
          <button
            type="button"
            className="approval__deny"
            onClick={() => onAnswer(block.id, "deny")}
          >
            Deny
          </button>
        </div>
      )}
    </section>
  );
}
