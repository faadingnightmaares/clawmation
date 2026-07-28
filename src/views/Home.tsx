import { useEffect, useState, type ComponentType } from "react";
import {
  ArrowTrendingUpIcon,
  ChartBarIcon,
  ClockIcon,
  LinkIcon,
  PlayCircleIcon,
  QueueListIcon,
  ShieldCheckIcon,
} from "@heroicons/react/24/outline";
import {
  IconArrowRight,
  IconCheck,
  IconCircleDot,
  IconListDetails,
} from "@tabler/icons-react";

import {
  getRunHistory,
  getStatsSummary,
  type HistoryEntry,
  type StatsSummary,
} from "@/api";
import { fmtAgo, fmtDur } from "@/format";
import { cn } from "@/lib/utils";
import { VIEW_ICONS, VIEW_ICON_STROKE_WIDTH } from "@/nav";
import { AntiAfkCard } from "@/components/AntiAfkCard";
import { Skeleton } from "@/components/ui/skeleton";
import type { ViewProps } from "./types";

const PANEL =
  "rounded-[18px] border border-border/80 bg-card shadow-[0_12px_34px_rgba(50,35,18,0.045)]";

type HomeIcon = ComponentType<{
  className?: string;
  strokeWidth?: number;
  "aria-hidden"?: boolean;
}>;

const MacrosIcon = VIEW_ICONS.macros;
const WatchIcon = VIEW_ICONS.vision;
const LoopsIcon = VIEW_ICONS.nodes;

export function Home({ status, navigate }: ViewProps) {
  const [stats, setStats] = useState<StatsSummary | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [statsResult, historyResult] = await Promise.allSettled([
        getStatsSummary(),
        getRunHistory(6),
      ]);
      if (!alive) return;
      if (statsResult.status === "fulfilled") setStats(statsResult.value);
      if (historyResult.status === "fulfilled") {
        setHistory(historyResult.value);
      }
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, []);

  const hour = new Date().getHours();
  const greeting =
    hour < 12
      ? "Good morning"
      : hour < 18
        ? "Good afternoon"
        : "Good evening";
  const subtitle =
    status?.mode === "recording"
      ? "Your macro is recording now."
      : status?.mode === "playing"
        ? `Playing ${status.last_macro.trim() || "your macro"} now.`
        : status?.mode === "paused"
          ? "Your macro is paused."
          : "Pick up where you left off, or start something new.";

  return (
    <div className="space-y-5 pb-4">
      <section
        aria-label="Workspace overview"
        className={cn(
          PANEL,
          "grid grid-cols-2 overflow-hidden lg:grid-cols-4",
        )}
      >
        <Metric
          icon={QueueListIcon}
          label="Macros"
          value={stats?.total_macros ?? 0}
          detail="Total recorded"
          loading={loading}
        />
        <Metric
          icon={ShieldCheckIcon}
          label="Guards"
          value={stats?.total_guards ?? 0}
          detail="Configured"
          loading={loading}
        />
        <Metric
          icon={LinkIcon}
          label="Chains"
          value={stats?.total_chains ?? 0}
          detail="Created"
          loading={loading}
        />
        <Metric
          icon={PlayCircleIcon}
          label="Plays"
          value={stats?.total_plays ?? 0}
          detail="All time"
          loading={loading}
        />
      </section>

      <div className="grid items-stretch gap-5 lg:grid-cols-[minmax(0,1.9fr)_minmax(300px,0.84fr)]">
        <div
          data-testid="home-primary-column"
          className="flex h-full flex-col gap-5"
        >
          <section className={cn(PANEL, "overflow-hidden")}>
            <div className="min-h-[164px] px-6 py-7 sm:px-7">
              <div className="max-w-xl">
                <h1 className="text-[26px] font-semibold tracking-[-0.035em] text-foreground">
                  {greeting}
                </h1>
                <p className="mt-1.5 max-w-md text-sm leading-6 text-muted-foreground">
                  {subtitle}
                </p>
              </div>

            </div>

            <div
              aria-label="Quick actions"
              className="relative z-[2] grid border-t border-border/70 bg-card/95 sm:grid-cols-3"
            >
              <QuickAction
                icon={MacrosIcon}
                title="Record macro"
                detail="Capture clicks and keys"
                onClick={() => navigate("macros")}
              />
              <QuickAction
                icon={WatchIcon}
                title="Watch screen"
                detail="Detect and respond"
                onClick={() => navigate("vision")}
              />
              <QuickAction
                icon={LoopsIcon}
                title="Loops"
                detail="Build reusable workflows"
                onClick={() => navigate("nodes")}
              />
            </div>
          </section>

          <AntiAfkCard />

          <button
            type="button"
            onClick={() => navigate("nodes")}
            className={cn(
              PANEL,
              "group mt-auto grid w-full gap-5 overflow-hidden p-6 text-left outline-none transition-[border-color,box-shadow,transform] duration-150 ease-[cubic-bezier(0.16,1,0.3,1)] hover:border-primary/35 active:translate-y-px focus-visible:ring-[3px] focus-visible:ring-ring/35 md:grid-cols-[auto_1fr_auto] md:items-center md:px-7",
            )}
          >
            <span className="flex size-12 items-center justify-center rounded-[14px] bg-primary/12 text-primary">
              <LoopsIcon
                className="size-6"
                strokeWidth={VIEW_ICON_STROKE_WIDTH}
              />
            </span>
            <span>
              <span className="block text-base font-semibold text-foreground">
                Build with Loops
              </span>
              <span className="mt-1 block max-w-2xl text-sm leading-6 text-muted-foreground">
                Connect macros, waits, vision checks, and actions into one
                reusable workflow.
              </span>
            </span>
            <span className="flex items-center gap-2 text-sm font-semibold text-primary">
              Open workspace
              <IconArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
            </span>
          </button>
        </div>

        <aside
          data-testid="home-sidebar"
          className="grid h-full gap-5 lg:grid-rows-[auto_minmax(0,1fr)]"
        >
          <Overview stats={stats} loading={loading} />
          <RecentActivity
            history={history}
            loading={loading}
            onViewMacros={() => navigate("macros")}
          />
        </aside>
      </div>
    </div>
  );
}

function Metric({
  icon: IconComponent,
  label,
  value,
  detail,
  loading,
}: {
  icon: HomeIcon;
  label: string;
  value: number;
  detail: string;
  loading: boolean;
}) {
  return (
    <div className="flex min-h-[104px] items-center gap-4 border-border/75 p-5 even:border-l lg:border-l lg:first:border-l-0">
      <span className="flex size-11 shrink-0 items-center justify-center rounded-[14px] bg-primary/10 text-primary">
        <IconComponent
          className="size-[22px]"
          strokeWidth={VIEW_ICON_STROKE_WIDTH}
          aria-hidden={true}
        />
      </span>
      <span className="min-w-0">
        <span className="block text-xs font-medium text-muted-foreground">
          {label}
        </span>
        {loading ? (
          <Skeleton className="my-1 h-7 w-10" />
        ) : (
          <span className="mt-0.5 block text-2xl font-semibold tabular-nums tracking-tight text-foreground">
            {value}
          </span>
        )}
        <span className="block text-[11px] text-muted-foreground/80">
          {detail}
        </span>
      </span>
    </div>
  );
}

function QuickAction({
  icon: IconComponent,
  title,
  detail,
  onClick,
}: {
  icon: HomeIcon;
  title: string;
  detail: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group flex min-h-[88px] items-center gap-3 border-border/70 px-5 text-left outline-none transition-[color,background-color,transform] duration-150 ease-[cubic-bezier(0.16,1,0.3,1)] hover:bg-muted/45 active:translate-y-px focus-visible:bg-muted/45 focus-visible:ring-[3px] focus-visible:ring-inset focus-visible:ring-ring/30 sm:even:border-l lg:border-l lg:first:border-l-0"
    >
      <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
        <IconComponent
          className="size-[18px]"
          strokeWidth={VIEW_ICON_STROKE_WIDTH}
          aria-hidden={true}
        />
      </span>
      <span className="min-w-0">
        <span className="block text-xs font-semibold text-foreground">
          {title}
        </span>
        <span className="mt-1 block text-[11px] leading-4 text-muted-foreground">
          {detail}
        </span>
      </span>
    </button>
  );
}

function Overview({
  stats,
  loading,
}: {
  stats: StatsSummary | null;
  loading: boolean;
}) {
  const mostPlayed =
    stats?.most_played?.trim() || (loading ? "" : "No runs yet");

  return (
    <section className={cn(PANEL, "p-5")} aria-labelledby="overview-title">
      <div className="flex items-center gap-2">
        <ArrowTrendingUpIcon className="size-[18px] text-primary" />
        <h2 id="overview-title" className="text-sm font-semibold text-foreground">
          Overview
        </h2>
      </div>

      <dl className="mt-4 space-y-1.5">
        <OverviewRow
          icon={PlayCircleIcon}
          label="Total plays"
          value={loading ? null : String(stats?.total_plays ?? 0)}
        />
        <OverviewRow
          icon={QueueListIcon}
          label="Macros used"
          value={loading ? null : String(stats?.macros_played ?? 0)}
        />
        <OverviewRow
          icon={ChartBarIcon}
          label="Most used"
          value={loading ? null : mostPlayed}
        />
      </dl>
    </section>
  );
}

function OverviewRow({
  icon: IconComponent,
  label,
  value,
}: {
  icon: HomeIcon;
  label: string;
  value: string | null;
}) {
  return (
    <div className="grid min-h-11 grid-cols-[auto_1fr_minmax(0,1.2fr)] items-center gap-2 rounded-xl bg-muted/35 px-3">
      <IconComponent
        className="size-4 text-muted-foreground"
        strokeWidth={VIEW_ICON_STROKE_WIDTH}
      />
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="truncate text-right text-xs font-semibold text-foreground">
        {value === null ? (
          <Skeleton className="ml-auto h-4 w-12" />
        ) : (
          value
        )}
      </dd>
    </div>
  );
}

function RecentActivity({
  history,
  loading,
  onViewMacros,
}: {
  history: HistoryEntry[];
  loading: boolean;
  onViewMacros: () => void;
}) {
  return (
    <section
      className={cn(PANEL, "h-full p-5")}
      aria-labelledby="recent-activity-title"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-2">
          <ClockIcon className="size-[18px] text-primary" />
          <h2
            id="recent-activity-title"
            className="text-sm font-semibold text-foreground"
          >
            Recent activity
          </h2>
        </div>
        <button
          type="button"
          onClick={onViewMacros}
          className="rounded-md text-xs font-medium text-primary outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/30"
        >
          View macros
        </button>
      </div>

      {loading ? (
        <div className="mt-4 space-y-2">
          {[0, 1, 2, 3].map((item) => (
            <Skeleton key={item} className="h-[58px] w-full rounded-xl" />
          ))}
        </div>
      ) : history.length ? (
        <div className="mt-4 space-y-1.5">
          {history.map((entry) => (
            <RunRow key={`${entry.timestamp}-${entry.name}`} entry={entry} />
          ))}
        </div>
      ) : (
        <div className="mt-4 rounded-[14px] bg-muted/35 px-5 py-8 text-center">
          <IconCircleDot className="mx-auto size-7 text-primary" />
          <p className="mt-3 text-sm font-semibold text-foreground">
            No activity yet
          </p>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            Completed runs will appear here.
          </p>
        </div>
      )}
    </section>
  );
}

function RunRow({ entry }: { entry: HistoryEntry }) {
  const completed = entry.status === "" || entry.status === "completed";
  const running = entry.status === "running";

  return (
    <div className="grid min-h-[58px] grid-cols-[auto_1fr_auto] items-center gap-3 rounded-xl bg-muted/30 px-3">
      <span className="flex size-8 items-center justify-center rounded-lg bg-card text-muted-foreground shadow-[inset_0_0_0_1px_var(--border)]">
        <IconListDetails className="size-4" />
      </span>
      <span className="min-w-0">
        <span className="block truncate text-xs font-semibold text-foreground">
          {entry.name}
        </span>
        <span className="mt-0.5 block text-[11px] text-muted-foreground">
          {fmtAgo(entry.timestamp)}
          {entry.duration > 0 && ` · ${fmtDur(entry.duration)}`}
        </span>
      </span>
      <span
        className={cn(
          "flex items-center gap-1 text-[11px] font-medium",
          completed
            ? "text-success"
            : running
              ? "text-primary"
              : "text-muted-foreground",
        )}
      >
        {completed && <IconCheck className="size-3.5" strokeWidth={2.5} />}
        {running ? "Playing" : completed ? "Completed" : "Stopped"}
      </span>
    </div>
  );
}
