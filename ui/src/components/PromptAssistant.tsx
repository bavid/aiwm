import { useEffect, useState } from "react";
import { useModels } from "../lib/hooks";
import { cancelJob, jobDetail, submitJob, type JobState } from "../lib/ipc";
import {
  buildTranscriptPrompt,
  extractSuggestion,
  type AssistantKind,
  type AssistantTurn,
} from "../lib/prompt-assistant";
import "./prompt-assistant.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];

/** A collapsible back-and-forth with a local chat model to help write an
 *  image/video prompt -- collapsed by default so it doesn't compete with the
 *  generation form itself. The underlying chat endpoint has no server-side
 *  memory (one stateless message per request), so each turn re-sends the
 *  whole transcript so far (see `buildTranscriptPrompt`). */
export function PromptAssistant({
  kind,
  sessionId,
  onApplyPrompt,
  onApplyNegative,
}: {
  kind: AssistantKind;
  sessionId: string | null;
  onApplyPrompt: (text: string) => void;
  onApplyNegative?: (text: string) => void;
}) {
  const { data: models } = useModels();
  const chatModels = (models ?? []).filter((m) => m.roles.includes("chat"));

  const [open, setOpen] = useState(false);
  const [modelId, setModelId] = useState("auto");
  const [history, setHistory] = useState<AssistantTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [streaming, setStreaming] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    const tick = async () => {
      let detail;
      try {
        detail = await jobDetail(pendingId);
      } catch {
        return;
      }
      if (!alive || !detail) return;
      const { job } = detail;
      if (!DONE.includes(job.state)) {
        // `blocked` (not enough VRAM right now) isn't terminal -- the
        // scheduler re-checks every tick and can recover on its own -- so
        // keep polling, just surface why instead of a silent "..." forever.
        setError(job.state === "blocked" ? job.error_text ?? "not enough VRAM free right now" : null);
        setStreaming(job.result ?? "");
        return;
      }
      setPendingId(null);
      setStreaming("");
      if (job.state === "completed") {
        setHistory((h) => [...h, { role: "assistant", text: job.result ?? "" }]);
      } else if (job.state === "failed") {
        setError(job.error_text ?? "the assistant failed to answer");
      }
    };
    tick();
    const id = setInterval(tick, 400);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId]);

  const send = async () => {
    const text = draft.trim();
    if (!text || pendingId) return;
    setError(null);
    const fullPrompt = buildTranscriptPrompt(kind, history, text);
    setHistory((h) => [...h, { role: "user", text }]);
    setDraft("");
    try {
      const picked = modelId === "auto" ? null : chatModels.find((m) => m.id === modelId);
      const job = await submitJob({
        job_type: "chat",
        model_id: picked ? picked.id : undefined,
        runtime_id: picked ? "llamacpp" : undefined,
        params: { prompt: fullPrompt, max_tokens: 400 },
        session_id: sessionId ?? undefined,
      });
      setPendingId(job.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const reset = () => {
    if (pendingId) cancelJob(pendingId);
    setPendingId(null);
    setStreaming("");
    setHistory([]);
    setError(null);
  };

  return (
    <div className="prompt-assistant">
      <button
        type="button"
        className="prompt-assistant__toggle"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {open ? "▾" : "▸"} Prompt assistant
        <span className="muted"> — talk through what you want, it drafts the prompt</span>
      </button>

      {open && (
        <div className="prompt-assistant__panel">
          <div className="prompt-assistant__head">
            <select value={modelId} onChange={(e) => setModelId(e.target.value)}>
              <option value="auto">Auto (most-recently-used)</option>
              {chatModels.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
            {(history.length > 0 || pendingId) && (
              <button type="button" className="chip" onClick={reset}>
                {pendingId ? "Cancel" : "New chat"}
              </button>
            )}
          </div>

          <div className="prompt-assistant__log">
            {history.length === 0 && !pendingId && (
              <p className="muted">
                Describe what you're after -- it'll ask a question or two, then suggest a prompt.
              </p>
            )}
            {history.map((turn, i) => (
              <AssistantBubble
                key={i}
                turn={turn}
                onApplyPrompt={onApplyPrompt}
                onApplyNegative={onApplyNegative}
              />
            ))}
            {pendingId && (
              <div className="prompt-assistant__bubble prompt-assistant__bubble--assistant">
                {streaming || "…"}
              </div>
            )}
          </div>
          {error && <p className="prompt-assistant__err">{error}</p>}

          <form
            className="prompt-assistant__composer"
            onSubmit={(e) => {
              e.preventDefault();
              send();
            }}
          >
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder={
                kind === "edit"
                  ? "e.g. remove the blisters, or make me look like Neo"
                  : "e.g. a moody portrait of an old lighthouse keeper"
              }
              spellCheck
            />
            <button type="submit" disabled={!draft.trim() || !!pendingId}>
              {pendingId ? "…" : "Send"}
            </button>
          </form>
        </div>
      )}
    </div>
  );
}

function AssistantBubble({
  turn,
  onApplyPrompt,
  onApplyNegative,
}: {
  turn: AssistantTurn;
  onApplyPrompt: (text: string) => void;
  onApplyNegative?: (text: string) => void;
}) {
  const suggestion = turn.role === "assistant" ? extractSuggestion(turn.text) : {};
  const className =
    turn.role === "user"
      ? "prompt-assistant__bubble prompt-assistant__bubble--user"
      : "prompt-assistant__bubble prompt-assistant__bubble--assistant";

  return (
    <div className={className}>
      <p>{turn.text}</p>
      {(suggestion.prompt || suggestion.negative) && (
        <div className="prompt-assistant__suggestion">
          {suggestion.prompt && (
            <button type="button" className="chip" onClick={() => onApplyPrompt(suggestion.prompt!)}>
              Use this prompt
            </button>
          )}
          {suggestion.negative && onApplyNegative && (
            <button
              type="button"
              className="chip"
              onClick={() => onApplyNegative(suggestion.negative!)}
            >
              Use this negative
            </button>
          )}
        </div>
      )}
    </div>
  );
}
