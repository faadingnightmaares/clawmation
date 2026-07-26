import { useEffect, useRef, useState } from "react";
import { animate } from "animejs";
import { Keyboard, X } from "lucide-react";

import { hotkeysResume, hotkeysSuspend } from "@/api";
import { accelCaps, accelFromEvent, isModifierOnly } from "@/lib/hotkeys";
import { reducedMotion } from "@/lib/anime";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";

interface HotkeyFieldProps {
  id: string;
  label: string;
  value: string;
  /** Called with the new shortcut, or `""` to unbind. */
  onCapture: (accel: string) => void;
}

/**
 * Press-to-set shortcut field: click it, press the keys, done. No typing the
 * name of a key. Escape backs out, Backspace unbinds.
 *
 * While it listens, the global shortcuts are released (`hotkeys_suspend`),
 * because Tauri's are exclusive: pressing the key that's already bound to
 * record would start a recording instead of being captured here.
 */
export function HotkeyField({ id, label, value, onCapture }: HotkeyFieldProps) {
  const [listening, setListening] = useState(false);
  const capsRef = useRef<HTMLSpanElement>(null);
  const onCaptureRef = useRef(onCapture);
  // Whether the current listening session committed a value (a real accel or an
  // unbind). When it did, `update_config` re-registers the shortcuts itself, so
  // the effect cleanup must NOT also fire `hotkeysResume` — the two register
  // concurrently and the resume can re-register the *old* shortcut on top of the
  // new one, leaving the change dead until a restart. Resume runs only on the
  // cancel path (Esc / blur / unmount), where no save happened and the suspend
  // left the shortcuts released.
  const committedRef = useRef(false);

  useEffect(() => {
    onCaptureRef.current = onCapture;
  }, [onCapture]);

  useEffect(() => {
    if (!listening) return;
    committedRef.current = false;
    // Best-effort: outside Tauri there are no global shortcuts to release, and
    // failing to release them must not stop the user setting a key.
    void hotkeysSuspend().catch(() => {});

    const onKey = (e: KeyboardEvent) => {
      // Swallow the keystroke before anything else in the app sees it: Alt+1..7
      // navigation, form submits, browser defaults.
      e.preventDefault();
      e.stopPropagation();
      if (isModifierOnly(e.code)) return;

      const bare = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
      if (e.code === "Escape" && bare) {
        setListening(false);
        return;
      }
      if ((e.code === "Backspace" || e.code === "Delete") && bare) {
        committedRef.current = true;
        onCaptureRef.current("");
        setListening(false);
        return;
      }
      const accel = accelFromEvent(e);
      if (!accel) {
        notify("info", "That key can’t be used as a shortcut. Try another.");
        return;
      }
      committedRef.current = true;
      onCaptureRef.current(accel);
      setListening(false);
    };

    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      // Re-arm only when the session backed out without saving: a committed
      // capture is re-registered by `update_config`, and a concurrent resume
      // here would race it back onto the old shortcut (see committedRef above).
      if (!committedRef.current) void hotkeysResume().catch(() => {});
    };
  }, [listening]);

  // Pop the caps when a new shortcut lands, so the change is visible even
  // though the field never showed a caret.
  useEffect(() => {
    const el = capsRef.current;
    if (!el || !value || reducedMotion()) return;
    animate(el, { scale: [0.82, 1], opacity: [0, 1], duration: 320, ease: "out(3)" });
  }, [value]);

  const caps = accelCaps(value);

  return (
    <div className="relative">
      <button
        id={id}
        type="button"
        aria-label={listening ? `${label}: press the keys you want` : `${label}: ${value || "not set"}`}
        data-listening={listening}
        onClick={() => setListening((l) => !l)}
        onBlur={() => setListening(false)}
        className={cn(
          "flex h-9 w-full items-center gap-2 rounded-md border border-input bg-transparent px-3 text-left text-sm shadow-xs transition-[color,box-shadow,border-color] outline-none",
          "hover:border-ring/60 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
          "data-[listening=true]:border-primary data-[listening=true]:ring-[3px] data-[listening=true]:ring-primary/25",
          "dark:bg-input/30 dark:hover:bg-input/50",
          value && !listening ? "pr-9" : "pr-3",
        )}
      >
        {listening ? (
          <span className="flex items-center gap-2 text-primary">
            <span className="relative flex size-2">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary opacity-60" />
              <span className="relative inline-flex size-2 rounded-full bg-primary" />
            </span>
            Press any key…
          </span>
        ) : caps.length > 0 ? (
          <span ref={capsRef} className="flex min-w-0 items-center gap-1">
            {caps.map((cap, i) => (
              <kbd
                key={`${cap}-${i}`}
                className="rounded border border-border bg-secondary/70 px-1.5 py-0.5 font-mono text-[11px] leading-none text-foreground"
              >
                {cap}
              </kbd>
            ))}
          </span>
        ) : (
          <span className="flex items-center gap-2 text-muted-foreground">
            <Keyboard className="size-4" />
            Click, then press a key
          </span>
        )}
      </button>

      {value && !listening && (
        <button
          type="button"
          aria-label={`Unbind ${label}`}
          title="Unbind"
          onClick={() => onCapture("")}
          className="absolute top-1/2 right-1 flex size-7 -translate-y-1/2 items-center justify-center rounded-md text-muted-foreground transition-colors outline-none hover:bg-secondary hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
        >
          <X className="size-3.5" />
        </button>
      )}
    </div>
  );
}
