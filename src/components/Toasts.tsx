import {
  createContext,
  useCallback,
  useContext,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

export type ToastKind = "info" | "success" | "error";

interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

interface ToastApi {
  /** Show a transient toast. Mirrors the Python UI's `pushToast(kind, text)`. */
  push: (kind: ToastKind, text: string) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

/** Auto-dismiss delay, matching the Python UI's `pushToast` (3400ms). */
const TOAST_MS = 3400;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(0);

  const push = useCallback((kind: ToastKind, text: string) => {
    const id = nextId.current++;
    setToasts((prev) => [...prev, { id, kind, text }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), TOAST_MS);
  }, []);

  return (
    <ToastContext.Provider value={{ push }}>
      {children}
      <ToastHost toasts={toasts} />
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used within a ToastProvider");
  }
  return ctx;
}

function accent(kind: ToastKind): string {
  switch (kind) {
    case "success":
      return "var(--verdict-allowed)";
    case "error":
      return "var(--verdict-blocked)";
    default:
      return "var(--color-gold)";
  }
}

const hostStyle: CSSProperties = {
  position: "fixed",
  right: 20,
  bottom: 20,
  zIndex: 9998,
  display: "flex",
  flexDirection: "column",
  gap: 8,
  pointerEvents: "none",
};

function ToastHost({ toasts }: { toasts: Toast[] }) {
  return (
    <div style={hostStyle}>
      {toasts.map((t) => (
        <div
          key={t.id}
          style={{
            minWidth: 220,
            maxWidth: 360,
            padding: "10px 14px",
            borderRadius: "var(--radius-md)",
            border: "1px solid var(--border-default)",
            borderLeft: `3px solid ${accent(t.kind)}`,
            background: "var(--surface-card-strong)",
            boxShadow: "var(--shadow-menu)",
            fontFamily: "var(--font-sans)",
            fontSize: "var(--text-base)",
            lineHeight: 1.5,
            color: "var(--text-78)",
          }}
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}
