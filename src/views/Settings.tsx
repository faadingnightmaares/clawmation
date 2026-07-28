import { useEffect, useState, type ReactNode } from "react";
import {
  ArrowClockwise,
  FolderOpen,
  GearSix,
  HardDrives,
  Keyboard,
  SpinnerGap,
  type Icon,
} from "@phosphor-icons/react";

import {
  checkUpdate,
  getConfig,
  getDataPaths,
  getVersion,
  hotkeysResume,
  installUpdate,
  onUpdateProgress,
  openDataFolder,
  updateConfig,
  type ConfigDto,
  type ConfigSaveResult,
  type DataPaths,
  type UpdateInfo,
} from "@/api";
import { HotkeyField } from "@/components/HotkeyField";
import { ReleaseUpdateDialog } from "@/components/ReleaseUpdateDialog";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { accelCaps } from "@/lib/hotkeys";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Guide, GUIDE_TOPICS, type GuideTopic } from "./Guide";
import type { ViewProps } from "./types";

type HotkeyKey = "hotkey_record" | "hotkey_play" | "hotkey_stop";
type SwitchKey = "indicator_on_top" | "humanize_clicks";
type SettingsPage =
  | "general"
  | "shortcuts"
  | "storage"
  | GuideTopic;

const DEFAULT_CONFIG: ConfigDto = {
  capture_backend: "",
  hotkey_record: "",
  hotkey_play: "",
  hotkey_stop: "",
  indicator_on_top: true,
  humanize_clicks: false,
  notify_on_schedule: false,
  notify_on_complete: false,
};

const HOTKEYS: { key: HotkeyKey; label: string; detail: string }[] = [
  {
    key: "hotkey_record",
    label: "Start or stop recording",
    detail: "Capture a new macro without returning to Clawmation.",
  },
  {
    key: "hotkey_play",
    label: "Play the last macro",
    detail: "Start your most recently selected recording.",
  },
  {
    key: "hotkey_stop",
    label: "Stop everything",
    detail: "Immediately stop playback, Loops, Watch actions, or recording.",
  },
];

const SETTINGS_PAGES: {
  id: Exclude<SettingsPage, GuideTopic>;
  label: string;
  Icon: Icon;
}[] = [
  { id: "general", label: "General", Icon: GearSix },
  { id: "shortcuts", label: "Shortcuts", Icon: Keyboard },
  { id: "storage", label: "Storage", Icon: HardDrives },
];

const prettyAccel = (accel: string) => accelCaps(accel).join(" + ");

function isGuideTopic(page: SettingsPage): page is GuideTopic {
  return GUIDE_TOPICS.some((topic) => topic.id === page);
}

export function Settings({ active = true, navigate }: ViewProps) {
  const [page, setPage] = useState<SettingsPage>("general");
  const [config, setConfig] = useState<ConfigDto>(DEFAULT_CONFIG);
  const [paths, setPaths] = useState<DataPaths | null>(null);
  const [version, setVersion] = useState("");
  const [loading, setLoading] = useState(true);
  const [checking, setChecking] = useState(false);
  const [available, setAvailable] = useState<UpdateInfo | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [cfg, dp, ver] = await Promise.allSettled([
        getConfig(),
        getDataPaths(),
        getVersion(),
      ]);
      if (!alive) return;
      if (cfg.status === "fulfilled") setConfig(cfg.value);
      if (dp.status === "fulfilled") setPaths(dp.value);
      if (ver.status === "fulfilled") setVersion(ver.value.version);
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, []);

  const persist = async (
    patch: Record<string, unknown>,
  ): Promise<ConfigSaveResult | null> => {
    try {
      const result = await updateConfig(patch);
      if (result.ok) return result;
    } catch {
      // Shared failure feedback below.
    }
    notify("error", "Couldn’t save that change. Please try again.");
    return null;
  };

  const setHotkey = async (key: HotkeyKey, accel: string) => {
    const previous = config[key];
    if (accel === previous) return;

    const clash = HOTKEYS.find(
      (hotkey) => hotkey.key !== key && accel && config[hotkey.key] === accel,
    );
    if (clash) {
      void hotkeysResume();
      notify(
        "error",
        `${prettyAccel(accel)} is already set to “${clash.label.toLowerCase()}”.`,
      );
      return;
    }

    setConfig((current) => ({ ...current, [key]: accel }));
    const result = await persist({ [key]: accel });
    if (!result) {
      void hotkeysResume();
      setConfig((current) => ({ ...current, [key]: previous }));
      return;
    }
    if (accel && result.unbound?.includes(accel)) {
      notify(
        "error",
        `Windows wouldn’t hand over ${prettyAccel(accel)}. Another app has it.`,
      );
    } else {
      notify(
        "success",
        accel ? `Shortcut set to ${prettyAccel(accel)}.` : "Shortcut removed.",
      );
    }
  };

  const toggle = async (key: SwitchKey, value: boolean) => {
    setConfig((current) => ({ ...current, [key]: value }));
    if (!(await persist({ [key]: value }))) {
      setConfig((current) => ({ ...current, [key]: !value }));
    }
  };

  const openFolder = async (kind: string) => {
    try {
      const result = await openDataFolder(kind);
      if (!result.ok) notify("error", "Couldn’t open that folder.");
    } catch {
      notify("error", "Couldn’t open that folder.");
    }
  };

  const runUpdateCheck = async () => {
    setChecking(true);
    try {
      const result = await checkUpdate();
      if (result.update_available) {
        setAvailable(result);
      } else {
        notify("success", "You’re up to date.");
      }
    } catch {
      notify("error", "Couldn’t check for updates right now.");
    } finally {
      setChecking(false);
    }
  };

  const runInstall = async () => {
    setInstalling(true);
    setProgress(null);
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await onUpdateProgress(([done, total]) => {
        if (total) setProgress(Math.min(100, Math.round((done / total) * 100)));
      });
      await installUpdate();
    } catch {
      notify("error", "The update couldn’t be installed. Try again in a moment.");
      setInstalling(false);
      setAvailable(null);
    } finally {
      unlisten?.();
    }
  };

  return (
    <div className="space-y-6 pb-8">
      <header>
        <h1 className="text-2xl font-semibold tracking-[-0.03em] text-foreground">
          Settings
        </h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Configure Clawmation and find clear answers without leaving the app.
        </p>
      </header>

      <div className="overflow-hidden rounded-xl border border-border bg-card shadow-xs md:grid md:min-h-[690px] md:grid-cols-[224px_minmax(0,1fr)]">
        <aside className="border-b border-border bg-muted/20 p-3 md:border-r md:border-b-0 md:p-4">
          <SettingsNavigation page={page} onChange={setPage} />
        </aside>

        <main
          key={page}
          aria-labelledby={isGuideTopic(page) ? "guide-article-title" : "settings-page-title"}
          className="min-w-0 px-5 pt-7 sm:px-8 md:px-10"
        >
          {isGuideTopic(page) ? (
            <Guide topic={page} navigate={navigate} />
          ) : loading ? (
            <SettingsSkeleton />
          ) : (
            <SettingsPageContent
              page={page}
              config={config}
              paths={paths}
              version={version}
              checking={checking}
              onHotkey={setHotkey}
              onToggle={toggle}
              onOpenFolder={openFolder}
              onCheckUpdate={runUpdateCheck}
            />
          )}
        </main>
      </div>

      <ReleaseUpdateDialog
        info={active ? available : null}
        installing={installing}
        progress={progress}
        onDismiss={() => setAvailable(null)}
        onInstall={() => void runInstall()}
      />
    </div>
  );
}

function SettingsNavigation({
  page,
  onChange,
}: {
  page: SettingsPage;
  onChange: (page: SettingsPage) => void;
}) {
  return (
    <nav aria-label="Settings and documentation" className="space-y-5">
      <NavigationGroup label="Settings">
        {SETTINGS_PAGES.map(({ id, label, Icon: PageIcon }) => (
          <NavigationItem
            key={id}
            active={page === id}
            label={label}
            Icon={PageIcon}
            onClick={() => onChange(id)}
          />
        ))}
      </NavigationGroup>

      <NavigationGroup label="Documentation">
        {GUIDE_TOPICS.map(({ id, label, Icon: PageIcon }) => (
          <NavigationItem
            key={id}
            active={page === id}
            label={label}
            Icon={PageIcon}
            onClick={() => onChange(id)}
          />
        ))}
      </NavigationGroup>
    </nav>
  );
}

function NavigationGroup({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div>
      <p className="px-2 pb-1.5 text-[10px] font-semibold tracking-[0.12em] text-muted-foreground/80 uppercase">
        {label}
      </p>
      <div className="grid grid-cols-2 gap-1 sm:grid-cols-3 md:grid-cols-1">
        {children}
      </div>
    </div>
  );
}

function NavigationItem({
  active,
  label,
  Icon: ItemIcon,
  onClick,
}: {
  active: boolean;
  label: string;
  Icon: Icon;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      onClick={onClick}
      className={cn(
        "flex h-9 min-w-0 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] font-medium outline-none transition-[background-color,color,box-shadow] duration-150",
        "focus-visible:ring-[3px] focus-visible:ring-ring/45",
        active
          ? "bg-muted text-foreground"
          : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
      )}
    >
      <ItemIcon
        className="size-4 shrink-0"
        weight="regular"
        aria-hidden="true"
      />
      <span className="truncate">{label}</span>
    </button>
  );
}

interface SettingsPageContentProps {
  page: Exclude<SettingsPage, GuideTopic>;
  config: ConfigDto;
  paths: DataPaths | null;
  version: string;
  checking: boolean;
  onHotkey: (key: HotkeyKey, accel: string) => Promise<void>;
  onToggle: (key: SwitchKey, value: boolean) => Promise<void>;
  onOpenFolder: (kind: string) => Promise<void>;
  onCheckUpdate: () => Promise<void>;
}

function SettingsPageContent(props: SettingsPageContentProps) {
  switch (props.page) {
    case "shortcuts":
      return <ShortcutsSettings {...props} />;
    case "storage":
      return <StorageSettings {...props} />;
    default:
      return <GeneralSettings {...props} />;
  }
}

function GeneralSettings({
  config,
  version,
  checking,
  onToggle,
  onCheckUpdate,
}: SettingsPageContentProps) {
  return (
    <SettingsArticle
      title="General"
      intro="Everyday behavior for playback and the small run indicator."
    >
      <SettingsSection title="Playback">
        <SettingRow
          title="Natural click timing"
          description="Add tiny human-like timing variation to clicks without changing the recorded route."
        >
          <Switch
            id="humanize_clicks"
            aria-label="Natural click timing"
            checked={config.humanize_clicks}
            onCheckedChange={(value) => void onToggle("humanize_clicks", value)}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection title="Run indicator">
        <SettingRow
          title="Keep the status indicator above other windows"
          description="Show the compact playback status while Clawmation is active."
        >
          <Switch
            id="indicator_on_top"
            aria-label="Keep the status indicator above other windows"
            checked={config.indicator_on_top}
            onCheckedChange={(value) => void onToggle("indicator_on_top", value)}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection title="Application">
        <SettingRow
          title={`Clawmation${version ? ` v${version}` : ""}`}
          description="Check for a newer release. You will see the release notes before installing."
        >
          <Button
            variant="outline"
            onClick={() => void onCheckUpdate()}
            disabled={checking}
          >
            {checking ? (
              <SpinnerGap className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwise className="size-4" aria-hidden="true" />
            )}
            {checking ? "Checking..." : "Check for updates"}
          </Button>
        </SettingRow>
      </SettingsSection>
    </SettingsArticle>
  );
}

function ShortcutsSettings({ config, onHotkey }: SettingsPageContentProps) {
  return (
    <SettingsArticle
      title="Shortcuts"
      intro="Press a field, then enter the key combination. Escape cancels and Backspace removes it."
    >
      <SettingsSection title="Global controls">
        <div className="divide-y divide-border">
          {HOTKEYS.map((hotkey) => (
            <div
              key={hotkey.key}
              className="grid gap-3 py-5 first:pt-0 last:pb-0 sm:grid-cols-[minmax(0,1fr)_17rem] sm:items-center sm:gap-8"
            >
              <div>
                <Label
                  htmlFor={hotkey.key}
                  className="text-sm font-medium text-foreground"
                >
                  {hotkey.label}
                </Label>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {hotkey.detail}
                </p>
              </div>
              <HotkeyField
                id={hotkey.key}
                label={hotkey.label}
                value={config[hotkey.key]}
                onCapture={(accel) => void onHotkey(hotkey.key, accel)}
              />
            </div>
          ))}
        </div>
      </SettingsSection>

      <div className="flex items-start gap-3 border-l-2 border-primary/55 bg-primary/[0.035] px-4 py-3 text-xs leading-5 text-muted-foreground">
        <Keyboard className="mt-0.5 size-4 shrink-0 text-primary" />
        <p>
          Keep Stop unique and easy to reach. It is the same emergency control
          for macros, Loops, Watch actions, chains, and recording.
        </p>
      </div>
    </SettingsArticle>
  );
}

function StorageSettings({ paths, onOpenFolder }: SettingsPageContentProps) {
  return (
    <SettingsArticle
      title="Storage"
      intro="Your recordings and vision assets stay on this computer."
    >
      <div className="grid grid-cols-3 divide-x divide-border border-y border-border py-4">
        <StorageStat value={paths?.macro_count ?? 0} label="Macros" />
        <StorageStat value={paths?.template_count ?? 0} label="Pictures" />
        <StorageStat value={paths?.snapshot_count ?? 0} label="Snapshots" />
      </div>

      <SettingsSection title="Data folders">
        <div className="divide-y divide-border">
          <FolderRow
            label="Macros"
            description="Recordings and imported macro files."
            onOpen={() => void onOpenFolder("macros")}
          />
          <FolderRow
            label="Vision pictures"
            description="Images used by Watch, safeguards, and Loops."
            onOpen={() => void onOpenFolder("templates")}
          />
          <FolderRow
            label="Snapshots"
            description="Saved frames used while testing detections."
            onOpen={() => void onOpenFolder("snapshots")}
          />
          <FolderRow
            label="All Clawmation data"
            description={paths?.root || "The application data folder."}
            onOpen={() => void onOpenFolder("root")}
            monoDescription
          />
        </div>
      </SettingsSection>
    </SettingsArticle>
  );
}

function SettingsArticle({
  title,
  intro,
  children,
}: {
  title: string;
  intro: string;
  children: ReactNode;
}) {
  return (
    <article className="mx-auto w-full max-w-3xl pb-12">
      <header className="border-b border-border pb-7">
        <h2
          id="settings-page-title"
          className="text-[1.75rem] font-semibold tracking-[-0.035em] text-foreground"
        >
          {title}
        </h2>
        <p className="mt-2 max-w-2xl text-sm leading-6 text-muted-foreground">
          {intro}
        </p>
      </header>
      <div>{children}</div>
    </article>
  );
}

function SettingsSection({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="border-b border-border py-7 last:border-b-0">
      <h3 className="mb-5 text-sm font-semibold text-foreground">{title}</h3>
      {children}
    </section>
  );
}

function SettingRow({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-8 py-5 first:pt-0 last:pb-0">
      <div>
        <p className="text-sm font-medium text-foreground">{title}</p>
        <p className="mt-1 max-w-xl text-xs leading-5 text-muted-foreground">
          {description}
        </p>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function StorageStat({ value, label }: { value: number; label: string }) {
  return (
    <div className="px-4 text-center">
      <p className="text-xl font-semibold tabular-nums text-foreground">{value}</p>
      <p className="mt-0.5 text-[11px] text-muted-foreground">{label}</p>
    </div>
  );
}

function FolderRow({
  label,
  description,
  onOpen,
  monoDescription = false,
}: {
  label: string;
  description: string;
  onOpen: () => void;
  monoDescription?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-5 py-4 first:pt-0 last:pb-0">
      <div className="min-w-0">
        <p className="text-sm font-medium text-foreground">{label}</p>
        <p
          className={cn(
            "mt-1 truncate text-xs text-muted-foreground",
            monoDescription && "font-mono",
          )}
          title={monoDescription ? description : undefined}
        >
          {description}
        </p>
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={onOpen}
        aria-label={`Open ${label} folder`}
      >
        <FolderOpen className="size-4" aria-hidden="true" />
        Open
      </Button>
    </div>
  );
}

function SettingsSkeleton() {
  return (
    <div
      aria-label="Loading settings"
      className="mx-auto w-full max-w-3xl space-y-7 pb-12"
    >
      <div className="space-y-3 border-b border-border pb-7">
        <Skeleton className="h-8 w-40" />
        <Skeleton className="h-4 w-80 max-w-full" />
      </div>
      <div className="space-y-5 py-2">
        <Skeleton className="h-4 w-24" />
        <Skeleton className="h-16 w-full" />
        <Separator />
        <Skeleton className="h-16 w-full" />
      </div>
    </div>
  );
}
