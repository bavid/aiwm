import type { PermissionDecision } from "../../lib/ipc";
import type { Block } from "./transcript";

/** One transcript block — a text paragraph, a tool-call card, an inline
 *  approval prompt, an idle marker, or an error. */
export function TranscriptBlock({
  block,
  answered,
  onAnswer,
}: {
  block: Block;
  answered: boolean;
  onAnswer: (requestId: string, decision: PermissionDecision) => void;
}) {
  if (block.block === "text") {
    return <div className="tblock tblock--text">{block.text}</div>;
  }
  if (block.block === "idle") {
    return <div className="tblock tblock--idle">— idle —</div>;
  }
  if (block.block === "error") {
    return <div className="tblock tblock--error">{block.message}</div>;
  }
  if (block.block === "tool") {
    return (
      <div className="tool" data-status={block.status}>
        <div className="tool__head">
          <span className="tool__name">{block.name}</span>
          <span className="tool__status">{block.status}</span>
        </div>
        {block.command && <pre className="tool__cmd">{block.command}</pre>}
        {block.output && <pre className="tool__out">{block.output}</pre>}
      </div>
    );
  }
  return (
    <div className="approval" data-answered={answered}>
      <div className="approval__head">
        <span className="badge badge--warn">approval</span>
        <span className="approval__kind">{block.kind}</span>
      </div>
      <pre className="approval__cmd">{block.summary}</pre>
      <div className="approval__actions">
        <button
          type="button"
          className="approval__allow"
          disabled={answered}
          onClick={() => onAnswer(block.id, "allow_once")}
        >
          Allow once
        </button>
        {block.always && (
          <button
            type="button"
            disabled={answered}
            onClick={() => onAnswer(block.id, "allow_always")}
            title={`Always allow: ${block.always}`}
          >
            Always
          </button>
        )}
        <button
          type="button"
          className="approval__deny"
          disabled={answered}
          onClick={() => onAnswer(block.id, "deny")}
        >
          Deny
        </button>
      </div>
    </div>
  );
}
