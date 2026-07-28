import { toast } from "sonner";

export type ToastKind = "info" | "success" | "error" | "warning";

/** Only interrupt for feedback that needs attention. Routine confirmations are
 * already visible in the UI state that changed. */
export function notify(kind: ToastKind, text: string) {
  switch (kind) {
    case "error":
      return toast.error(text);
    case "warning":
      return toast.warning(text);
    default:
      return undefined;
  }
}

/** Passive announcements stay out of the workspace. Users can check updates
 * from Settings when they choose. */
export function notifyAction(
  _text: string,
  _label: string,
  _onClick: () => void,
) {
  return undefined;
}

/**
 * Report a destructive action that already happened, with a way back. The undo
 * rides the toast instead of a confirm dialog on the way in: deleting stays one
 * click, and only the rare mistake costs a second one.
 */
export function notifyUndo(text: string, onUndo: () => void) {
  return toast(text, {
    duration: 8000,
    action: { label: "Undo", onClick: onUndo },
  });
}
