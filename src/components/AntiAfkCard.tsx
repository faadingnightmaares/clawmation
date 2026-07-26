import { type CSSProperties, useEffect, useState } from "react";
import { Gamepad2 } from "lucide-react";

import {
  antiAfkGet,
  antiAfkListWindows,
  antiAfkUpdate,
  type AntiAfkAction,
  type AntiAfkState,
  type AntiAfkUpdate,
  type SelectableWindow,
} from "@/api";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";

const DEFAULT_STATE: AntiAfkState = {
  enabled: false,
  target_id: null,
  interval_min: 15,
  action: "random",
  status: "off",
  error: null,
};

export function AntiAfkCard() {
  const [state, setState] = useState<AntiAfkState>(DEFAULT_STATE);
  const [windows, setWindows] = useState<SelectableWindow[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [current, list] = await Promise.allSettled([
        antiAfkGet(),
        antiAfkListWindows(),
      ]);
      if (!alive) return;
      if (current.status === "fulfilled") setState(current.value);
      if (list.status === "fulfilled") setWindows(list.value);
      if (current.status === "rejected" || list.status === "rejected") {
        setLocalError("Anti-AFK is not available right now.");
      }
      setLoading(false);
    })();

    const statePoll = window.setInterval(() => {
      void antiAfkGet()
        .then((current) => {
          if (alive) setState(current);
        })
        .catch(() => {});
    }, 2_000);
    const windowPoll = window.setInterval(() => {
      void antiAfkListWindows()
        .then((list) => {
          if (alive) {
            setWindows(list);
            setLocalError(null);
          }
        })
        .catch(() => {
          if (alive) setLocalError("Could not read the open windows.");
        });
    }, 10_000);
    return () => {
      alive = false;
      window.clearInterval(statePoll);
      window.clearInterval(windowPoll);
    };
  }, []);

  const update = async (patch: AntiAfkUpdate) => {
    setSaving(true);
    try {
      const result = await antiAfkUpdate(patch);
      if (!result.ok || !result.state) {
        setLocalError(result.error ?? "Anti-AFK could not be updated.");
        return false;
      }
      setState(result.state);
      setLocalError(null);
      return true;
    } catch {
      setLocalError("Anti-AFK could not be updated.");
      return false;
    } finally {
      setSaving(false);
    }
  };

  const chooseTarget = async (targetId: string) => {
    const previous = state.target_id;
    setState((current) => ({ ...current, target_id: targetId }));
    if (!(await update({ target_id: targetId }))) {
      setState((current) => ({ ...current, target_id: previous }));
    }
  };

  const commitInterval = async (value: number[]) => {
    const minutes = value[0] ?? state.interval_min;
    const previous = state.interval_min;
    setState((current) => ({ ...current, interval_min: minutes }));
    if (!(await update({ interval_min: minutes }))) {
      setState((current) => ({ ...current, interval_min: previous }));
    }
  };

  const chooseAction = async (action: AntiAfkAction) => {
    const previous = state.action;
    setState((current) => ({ ...current, action }));
    if (!(await update({ action }))) {
      setState((current) => ({ ...current, action: previous }));
    }
  };

  const targetAvailable =
    state.target_id !== null && windows.some((candidate) => candidate.id === state.target_id);
  const message = statusMessage(state, targetAvailable, localError);
  const sliderProgress = ((state.interval_min - 1) / 19) * 100;

  return (
    <section className="rounded-2xl border border-border bg-card p-5">
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <Gamepad2 className="size-5" />
          </div>
          <div>
            <h2 className="text-sm font-semibold text-foreground">Anti-AFK</h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Briefly act in one game, then return to what you were doing.
            </p>
          </div>
        </div>
        <Switch
          aria-label="Anti-AFK"
          checked={state.enabled}
          disabled={loading || saving || !state.target_id}
          onCheckedChange={(enabled) => void update({ enabled })}
        />
      </div>

      <div className="mt-5 grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="anti-afk-target">Game or app</Label>
          <Select
            value={state.target_id ?? ""}
            disabled={loading || saving}
            onValueChange={(value) => void chooseTarget(value)}
          >
            <SelectTrigger id="anti-afk-target" aria-label="Game or app" className="w-full">
              <SelectValue placeholder={windows.length ? "Choose a running window" : "No windows found"} />
            </SelectTrigger>
            <SelectContent>
              {windows.map((candidate) => (
                <SelectItem key={candidate.id} value={candidate.id}>
                  {candidate.title} · PID {candidate.pid}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-2">
          <Label htmlFor="anti-afk-action">Action</Label>
          <Select
            value={state.action}
            disabled={loading || saving}
            onValueChange={(value) => void chooseAction(value as AntiAfkAction)}
          >
            <SelectTrigger
              id="anti-afk-action"
              aria-label="Anti-AFK action"
              className="w-full"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="jump">Jump</SelectItem>
              <SelectItem value="walk">Walk</SelectItem>
              <SelectItem value="camera">Camera nudge</SelectItem>
              <SelectItem value="random">Random mix</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="mt-5 rounded-xl border border-border/70 bg-muted/25 px-4 py-3.5">
        <div className="mb-3 flex items-center justify-between gap-3">
          <Label htmlFor="anti-afk-interval">Action interval</Label>
          <span className="rounded-full bg-primary/12 px-2.5 py-1 text-xs font-semibold tabular-nums text-primary">
            {state.interval_min} {state.interval_min === 1 ? "minute" : "minutes"}
          </span>
        </div>
        <input
          id="anti-afk-interval"
          type="range"
          aria-label="Anti-AFK interval"
          min={1}
          max={20}
          step={1}
          value={state.interval_min}
          disabled={loading || saving}
          className="anti-afk-slider"
          style={{ "--anti-afk-progress": `${sliderProgress}%` } as CSSProperties}
          onInput={(event) => {
            const minutes = Number(event.currentTarget.value);
            setState((current) => ({ ...current, interval_min: minutes }));
          }}
          onPointerUp={(event) =>
            void commitInterval([Number(event.currentTarget.value)])
          }
          onKeyUp={(event) =>
            void commitInterval([Number(event.currentTarget.value)])
          }
        />
        <div className="mt-1.5 flex justify-between text-[10px] font-medium uppercase tracking-wider text-muted-foreground/70">
          <span>1 min</span>
          <span>20 min</span>
        </div>
      </div>

      <p
        className={
          state.status === "error" || state.status === "target_unavailable" || localError
            ? "mt-4 text-xs text-destructive"
            : "mt-4 text-xs text-muted-foreground"
        }
      >
        {message}
      </p>
    </section>
  );
}

function statusMessage(
  state: AntiAfkState,
  targetAvailable: boolean,
  localError: string | null,
): string {
  if (localError) return localError;
  if (state.error) return state.error;
  if (state.status === "acting") return "Acting now, then returning you.";
  if (state.status === "target_unavailable" || (state.target_id && !targetAvailable)) {
    return "The selected window is unavailable. Choose it again when it reappears.";
  }
  if (state.enabled) {
    return `On · ${actionLabel(state.action)} every ${state.interval_min} minutes.`;
  }
  if (!state.target_id) return "Choose a game or app to turn Anti-AFK on.";
  return `Ready. Turning it on runs ${actionLabel(state.action).toLowerCase()} immediately.`;
}

function actionLabel(action: AntiAfkAction): string {
  if (action === "jump") return "Jump";
  if (action === "walk") return "Walk";
  if (action === "camera") return "Camera nudge";
  return "Random mix";
}
