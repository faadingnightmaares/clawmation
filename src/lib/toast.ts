import { toast } from "sonner";

export type ToastKind = "info" | "success" | "error" | "warning";

/**
 * One verb for transient feedback across every view, backed by sonner. Replaces
 * the old hand-rolled `pushToast(kind, text)` so callers read the same.
 */
export function notify(kind: ToastKind, text: string) {
  switch (kind) {
    case "success":
      return toast.success(text);
    case "error":
      return toast.error(text);
    case "warning":
      return toast.warning(text);
    default:
      return toast.info(text);
  }
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
