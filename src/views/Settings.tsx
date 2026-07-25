import { useEffect, useState, type ReactNode } from "react";
import {
  Bell,
  BookOpen,
  FolderOpen,
  Info,
  Keyboard,
  Loader2,
  RefreshCw,
  SlidersHorizontal,
  Sparkles,
} from "lucide-react";

import {
  checkUpdate,
  getConfig,
  getDataPaths,
  getVersion,
  installUpdate,
  onUpdateProgress,
  openDataFolder,
  updateConfig,
  type ConfigDto,
  type ConfigSaveResult,
  type DataPaths,
  type UpdateInfo,
} from "@/api";
import { useStaggerIn } from "@/lib/anime";
import { accelCaps } from "@/lib/hotkeys";
import { notify } from "@/lib/toast";
import { HotkeyField } from "@/components/HotkeyField";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Guide } from "./Guide";
import type { ViewProps } from "./types";

type HotkeyKey = "hotkey_record" | "hotkey_play" | "hotkey_stop";
type SwitchKey = "notify_on_complete" | "notify_on_schedule" | "humanize_clicks";

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

const HOTKEYS: { key: HotkeyKey; label: string }[] = [
  { key: "hotkey_record", label: "Start / stop recording" },
  { key: "hotkey_play", label: "Play the last macro" },
  { key: "hotkey_stop", label: "Stop everything" },
];

const prettyAccel = (accel: string) => accelCaps(accel).join(" + ");

/** Preferences, plus the Guide as a second tab. It is reading material rather
 *  than a place you work, so it lives here instead of taking a slot in the bar. */
export function Settings(props: ViewProps) {
  const [tab, setTab] = useState("preferences");
  const [config, setConfig] = useState<ConfigDto>(DEFAULT_CONFIG);
  const [paths, setPaths] = useState<DataPaths | null>(null);
  const [version, setVersion] = useState("");
  const [loading, setLoading] = useState(true);
  const [checking, setChecking] = useState(false);
  const [available, setAvailable] = useState<UpdateInfo | null>(null);
  const [installing, setInstalling] = useState(false);
  /** Percent downloaded, or `null` while the size is still unknown. */
  const [progress, setProgress] = useState<number | null>(null);

  const listRef = useStaggerIn<HTMLDivElement>(loading);

  useEffect(() => {
    let alive = true;
    void (async () => {
      // A plain browser has no Tauri seam, so each call can reject; allSettled
      // lets one failure fall back to defaults without sinking the others.
      const [cfg, dp, ver] = await Promise.allSettled([getConfig(), getDataPaths(), getVersion()]);
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

  const persist = async (patch: Record<string, unknown>): Promise<ConfigSaveResult | null> => {
    try {
      const r = await updateConfig(patch);
      if (r.ok) return r;
    } catch {
      /* fall through to the shared failure toast */
    }
    notify("error", "Couldn’t save that change. Please try again.");
    return null;
  };

  const setHotkey = async (key: HotkeyKey, accel: string) => {
    const previous = config[key];
    if (accel === previous) return;

    // Two actions on one key would leave the second silently unregistered, so
    // say so instead of saving a shortcut that can never fire.
    const clash = HOTKEYS.find((hk) => hk.key !== key && accel && config[hk.key] === accel);
    if (clash) {
      notify("error", `${prettyAccel(accel)} is already set to “${clash.label.toLowerCase()}”.`);
      return;
    }

    setConfig((c) => ({ ...c, [key]: accel }));
    const res = await persist({ [key]: accel });
    if (!res) {
      setConfig((c) => ({ ...c, [key]: previous }));
      return;
    }
    if (accel && res.unbound?.includes(accel)) {
      notify("error", `Windows wouldn’t hand over ${prettyAccel(accel)}. Another app has it.`);
    } else {
      notify("success", accel ? `Shortcut set to ${prettyAccel(accel)}.` : "Shortcut removed.");
    }
  };

  const toggle = async (key: SwitchKey, value: boolean) => {
    setConfig((c) => ({ ...c, [key]: value }));
    if (!(await persist({ [key]: value }))) setConfig((c) => ({ ...c, [key]: !value }));
  };

  const openFolder = async (kind: string) => {
    try {
      const r = await openDataFolder(kind);
      if (!r.ok) notify("error", "Couldn’t open that folder.");
    } catch {
      notify("error", "Couldn’t open that folder.");
    }
  };

  const runUpdateCheck = async () => {
    setChecking(true);
    try {
      const r = await checkUpdate();
      if (r.update_available) setAvailable(r);
      else notify("success", "You’re up to date.");
    } catch {
      notify("error", "Couldn’t check for updates right now.");
    } finally {
      setChecking(false);
    }
  };

  // `installUpdate` restarts the app on success, so the only way out of this
  // function is a failure; there is no success branch to write.
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
    <div className="space-y-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Shortcuts, alerts and where your things live, plus the guide, whenever you want a
          refresher.
        </p>
      </header>

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList className="w-full sm:w-auto">
          <TabsTrigger value="preferences" className="sm:px-6">
            <SlidersHorizontal />
            Preferences
          </TabsTrigger>
          <TabsTrigger value="guide" className="sm:px-6">
            <BookOpen />
            Guide
          </TabsTrigger>
        </TabsList>

        <TabsContent value="preferences" className="pt-4">
          {loading ? (
            <div className="space-y-6">
              {[0, 1, 2, 3, 4].map((i) => (
                <Card key={i} className="gap-0 p-6">
                  <div className="flex items-start gap-3">
                    <Skeleton className="size-9 rounded-lg" />
                    <div className="space-y-2">
                      <Skeleton className="h-4 w-36" />
                      <Skeleton className="h-3 w-56" />
                    </div>
                  </div>
                  <Skeleton className="mt-5 h-9 w-full" />
                </Card>
              ))}
            </div>
          ) : (
            <div ref={listRef} className="space-y-6">
              <Section
                icon={Keyboard}
                title="Shortcuts"
                hint="Keys you can press anywhere to run things hands-free. Click one, then press the keys. Esc backs out, Backspace unbinds."
              >
                <div className="grid gap-4 sm:grid-cols-3">
                  {HOTKEYS.map((hk) => (
                    <div key={hk.key} className="space-y-1.5">
                      <Label htmlFor={hk.key} className="text-xs font-normal text-muted-foreground">
                        {hk.label}
                      </Label>
                      <HotkeyField
                        id={hk.key}
                        label={hk.label}
                        value={config[hk.key]}
                        onCapture={(accel) => void setHotkey(hk.key, accel)}
                      />
                    </div>
                  ))}
                </div>
              </Section>

              <Section icon={Bell} title="Notifications" hint="Get a friendly heads-up when something happens.">
                <div className="space-y-4">
                  <SwitchRow
                    id="notify_on_complete"
                    title="Tell me when a macro finishes"
                    checked={config.notify_on_complete}
                    onChange={(v) => void toggle("notify_on_complete", v)}
                  />
                  <Separator />
                  <SwitchRow
                    id="notify_on_schedule"
                    title="Tell me when a scheduled run fires"
                    checked={config.notify_on_schedule}
                    onChange={(v) => void toggle("notify_on_schedule", v)}
                  />
                </div>
              </Section>

              <Section icon={Sparkles} title="Feel" hint="Little touches that make automation feel less robotic.">
                <SwitchRow
                  id="humanize_clicks"
                  title="Move the mouse the way a person would"
                  desc="Adds tiny natural motion and timing so clicks look human"
                  checked={config.humanize_clicks}
                  onChange={(v) => void toggle("humanize_clicks", v)}
                />
              </Section>

              <Section icon={FolderOpen} title="Your files" hint="Everything Clawmation saves stays right here on your PC.">
                <div className="space-y-4">
                  <div className="grid grid-cols-3 gap-3">
                    <Stat n={paths?.macro_count ?? 0} label="macros" />
                    <Stat n={paths?.template_count ?? 0} label="saved pictures" />
                    <Stat n={paths?.snapshot_count ?? 0} label="snapshots" />
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button variant="outline" size="sm" onClick={() => void openFolder("macros")}>
                      <FolderOpen className="size-4" /> Open macros folder
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => void openFolder("templates")}>
                      <FolderOpen className="size-4" /> Open pictures folder
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => void openFolder("snapshots")}>
                      <FolderOpen className="size-4" /> Open snapshots folder
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => void openFolder("root")}>
                      <FolderOpen className="size-4" /> Open data folder
                    </Button>
                  </div>
                  {paths?.root && (
                    <p className="truncate font-mono text-xs text-muted-foreground" title={paths.root}>
                      {paths.root}
                    </p>
                  )}
                </div>
              </Section>

              <Section icon={Info} title="About" hint="Check now and then for the latest fixes.">
                <div className="flex items-center justify-between gap-4">
                  <div className="space-y-0.5">
                    <p className="text-sm font-medium text-foreground">
                      Clawmation{version && ` v${version}`}
                    </p>
                    <p className="text-xs text-muted-foreground">Thanks for letting the cat help out.</p>
                  </div>
                  <Button variant="outline" size="sm" onClick={() => void runUpdateCheck()} disabled={checking}>
                    {checking ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
                    Check for updates
                  </Button>
                </div>
              </Section>
            </div>
          )}
        </TabsContent>

        <TabsContent value="guide" className="pt-4">
          <Guide {...props} />
        </TabsContent>
      </Tabs>

      {/* Closing mid-download would leave the installer running unattended, so
          the dialog only dismisses while the user still has a choice. */}
      <AlertDialog
        open={!!available}
        onOpenChange={(o) => !o && !installing && setAvailable(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Clawmation {available?.latest} is ready</AlertDialogTitle>
            <AlertDialogDescription>
              You’re on {available?.current}. Installing takes a moment and restarts the app, so
              finish anything that’s running first.
            </AlertDialogDescription>
          </AlertDialogHeader>

          {available?.notes && (
            <p className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">
              {available.notes}
            </p>
          )}

          {installing ? (
            <div className="space-y-2 py-2">
              <Progress value={progress ?? 0} />
              <p className="text-xs text-muted-foreground">
                {progress === null ? "Downloading…" : `Downloading… ${progress}%`}
              </p>
            </div>
          ) : (
            <AlertDialogFooter>
              <AlertDialogCancel>Not now</AlertDialogCancel>
              <AlertDialogAction onClick={(e) => (e.preventDefault(), void runInstall())}>
                Install and restart
              </AlertDialogAction>
            </AlertDialogFooter>
          )}
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function Section({
  icon: Icon,
  title,
  hint,
  children,
}: {
  icon: typeof Keyboard;
  title: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <Card className="gap-0 p-6">
      <div className="flex items-start gap-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Icon className="size-5" />
        </span>
        <div className="space-y-0.5">
          <h2 className="text-sm font-semibold text-foreground">{title}</h2>
          <p className="text-xs text-muted-foreground">{hint}</p>
        </div>
      </div>
      <div className="mt-5">{children}</div>
    </Card>
  );
}

function SwitchRow({
  id,
  title,
  desc,
  checked,
  onChange,
}: {
  id: string;
  title: string;
  desc?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="space-y-0.5">
        <Label htmlFor={id} className="text-sm font-medium text-foreground">
          {title}
        </Label>
        {desc && <p className="text-xs text-muted-foreground">{desc}</p>}
      </div>
      <Switch id={id} checked={checked} onCheckedChange={onChange} />
    </div>
  );
}

function Stat({ n, label }: { n: number; label: string }) {
  return (
    <div className="rounded-lg border border-border bg-secondary/40 p-4 text-center">
      <p className="text-2xl font-semibold tabular-nums text-foreground">{n}</p>
      <p className="text-xs text-muted-foreground">{label}</p>
    </div>
  );
}
