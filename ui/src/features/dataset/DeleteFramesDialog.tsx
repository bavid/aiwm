import { ConfirmDialog } from "../../components/ConfirmDialog";
import { framesLabel } from "./curation";

/** What the curator is about to delete, counted when the dialog opens. */
export type DeleteRequest = {
  ids: string[];
  fromKeep: number;
  fromDiscard: number;
  /** Selected frames not rendered right now (paged out or filtered away). */
  offscreen: number;
  isBusy: boolean;
  error: string | null;
};

type Props = {
  request: DeleteRequest | null;
  onConfirm: () => void;
  onCancel: () => void;
};

/** The irreversible frame deletion, spelled out: how many from each column,
 *  how many are not on screen, and a warning when kept frames are included.
 *  Frame rows carry no file size, so the freed size is reported afterwards. */
export function DeleteFramesDialog({ request, onConfirm, onCancel }: Props) {
  const count = request?.ids.length ?? 0;
  return (
    <ConfirmDialog
      isOpen={request !== null}
      title={`Delete ${framesLabel(count)}?`}
      confirmLabel={`Delete ${framesLabel(count)}`}
      tone="danger"
      isBusy={request?.isBusy ?? false}
      error={request?.error ?? null}
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      {request && (
        <>
          <p>
            <strong>
              {request.fromKeep.toLocaleString()} from Keep, {request.fromDiscard.toLocaleString()}{" "}
              from Discard
            </strong>
            {request.offscreen > 0 &&
              ` — ${request.offscreen.toLocaleString()} of them not shown on screen`}
            .
          </p>
          {request.fromKeep > 0 && (
            <p className="confirm__warn">
              This includes {framesLabel(request.fromKeep)} you are keeping for export.
            </p>
          )}
          <p>
            This cannot be undone: the frames' files are removed from disk; your source videos are
            not touched. The freed size is shown after deleting.
          </p>
          <p>To set frames aside without deleting them, move them to Discard instead.</p>
        </>
      )}
    </ConfirmDialog>
  );
}
