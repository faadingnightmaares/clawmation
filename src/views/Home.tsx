import { useEffect, useState } from "react";
import { ArrowRight, Check, CircleDot, Eye, ListVideo, ShieldCheck, Sparkles, Workflow, type LucideIcon } from "lucide-react";

import { getRunHistory, getStatsSummary, type HistoryEntry, type StatsSummary } from "@/api";
import { fmtAgo, fmtDur } from "@/format";
import { useStaggerIn } from "@/lib/anime";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { ViewProps } from "./types";

export function Home({ status, navigate }: ViewProps) {
  const [stats, setStats] = useState<StatsSummary | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);

  const pageRef = useStaggerIn<HTMLDivElement>();
  const listRef = useStaggerIn<HTMLDivElement>(history.length);

  useEffect(() => {
    let alive = true;
    void (async () => {
      // In a plain browser the Tauri bridge is absent and these throw; settle both
      // independently so one failing still lets the other render, and fall back to
      // an empty/zero state rather than an error.
      const [s, h] = await Promise.allSettled([getStatsSummary(), getRunHistory(8)]);
      if (!alive) return;
      if (s.status === "fulfilled") setStats(s.value);
      if (h.status === "fulfilled") setHistory(h.value);
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, []);

  const hour = new Date().getHours();
  const greeting = hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
  const subtitle =
    status?.mode === "recording"
      ? "Recording right now — press your stop key when you’re done."
      : status?.mode === "playing"
        ? `Playing ${status.last_macro.trim() || "a macro"} right now.`
        : status?.mode === "paused"
          ? "Paused for a moment — pick up whenever you like."
          : "Pick up where you left off, or start something new.";

  return (
    <div ref={pageRef} className="space-y-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">{greeting}</h1>
        <p className="text-sm text-muted-foreground">{subtitle}</p>
      </header>

      {/* The two things you'll reach for most */}
      <div className="grid gap-4 sm:grid-cols-2">
        <ActionCard
          icon={CircleDot}
          title="Record a macro"
          blurb="Capture your clicks and keystrokes, then replay them any time."
          onClick={() => navigate("macros")}
        />
        <ActionCard
          icon={Eye}
          title="Watch the screen"
          blurb="Let Clawmation keep an eye out and act on its own — no macro needed."
          onClick={() => navigate("vision")}
        />
      </div>

      {/* At a glance */}
      <section className="space-y-3">
        <div className="grid grid-cols-3 gap-3">
          <Stat icon={ListVideo} label="Macros" value={stats?.total_macros ?? 0} loading={loading} />
          <Stat icon={ShieldCheck} label="Guards" value={stats?.total_guards ?? 0} loading={loading} />
          <Stat icon={Workflow} label="Chains" value={stats?.total_chains ?? 0} loading={loading} />
        </div>
        {stats?.most_played && (
          <p className="text-xs text-muted-foreground">
            Most used: <span className="font-medium text-foreground">{stats.most_played}</span>
            {stats.most_played_count > 0 &&
              ` · played ${stats.most_played_count} ${stats.most_played_count === 1 ? "time" : "times"}`}
          </p>
        )}
      </section>

      {/* Recent activity */}
      <section className="space-y-4">
        <div>
          <h3 className="text-sm font-semibold text-foreground">Recent activity</h3>
          <p className="text-xs text-muted-foreground">The last few things that ran.</p>
        </div>

        {loading ? (
          <div className="space-y-2">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-[62px] w-full rounded-xl" />
            ))}
          </div>
        ) : history.length ? (
          <div ref={listRef} className="space-y-2">
            {history.map((h) => (
              <RunRow key={`${h.timestamp}-${h.name}`} entry={h} />
            ))}
          </div>
        ) : (
          <div className="flex flex-col items-center gap-4 rounded-2xl border border-dashed border-border px-6 py-14 text-center">
            <div className="flex size-12 items-center justify-center rounded-full bg-secondary text-muted-foreground">
              <Sparkles className="size-6" />
            </div>
            <div>
              <h4 className="text-base font-semibold text-foreground">Nothing’s run yet</h4>
              <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
                Record your first macro and it’ll show up here every time it plays.
              </p>
            </div>
            <Button onClick={() => navigate("macros")}>
              <CircleDot className="size-4" /> Record your first macro
            </Button>
          </div>
        )}
      </section>
    </div>
  );
}

function ActionCard({
  icon: Icon,
  title,
  blurb,
  onClick,
}: {
  icon: LucideIcon;
  title: string;
  blurb: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group flex items-center gap-4 rounded-2xl border border-border bg-card p-6 text-left transition-colors hover:border-primary/40"
    >
      <div className="flex size-12 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
        <Icon className="size-6" />
      </div>
      <div className="min-w-0 flex-1">
        <h3 className="text-base font-semibold text-foreground">{title}</h3>
        <p className="mt-0.5 text-sm text-muted-foreground">{blurb}</p>
      </div>
      <ArrowRight className="size-5 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-primary" />
    </button>
  );
}

function Stat({ icon: Icon, label, value, loading }: { icon: LucideIcon; label: string; value: number; loading: boolean }) {
  return (
    <div className="rounded-xl border border-border bg-card p-4">
      <div className="flex items-center gap-1.5 text-muted-foreground">
        <Icon className="size-4" />
        <span className="text-xs font-medium">{label}</span>
      </div>
      {loading ? (
        <Skeleton className="mt-2 h-8 w-10" />
      ) : (
        <p className="mt-1 text-3xl font-semibold tabular-nums text-foreground">{value}</p>
      )}
    </div>
  );
}

function RunRow({ entry }: { entry: HistoryEntry }) {
  const tone = runTone(entry.status);
  return (
    <div className="flex items-center gap-3 rounded-xl border border-border bg-card px-4 py-3">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-secondary text-muted-foreground">
        <ListVideo className="size-4" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-foreground">{entry.name}</p>
        <p className="text-xs text-muted-foreground">
          {fmtAgo(entry.timestamp)}
          {entry.duration > 0 && ` · took ${fmtDur(entry.duration)}`}
        </p>
      </div>
      <span className={cn("inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium", tone.cls)}>
        {tone.icon}
        {tone.label}
      </span>
    </div>
  );
}

function runTone(status: string): { label: string; cls: string; icon: React.ReactNode } {
  if (status === "running") return { label: "Playing…", cls: "bg-secondary text-muted-foreground", icon: null };
  if (status === "" || status === "completed")
    return { label: "Played", cls: "bg-primary/15 text-primary", icon: <Check className="size-3" /> };
  return { label: "Didn’t finish", cls: "bg-muted text-muted-foreground", icon: null };
}
