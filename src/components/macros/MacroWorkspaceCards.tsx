import { useEffect, useRef, useState } from "react";
import {
  IconBookmarkPlus,
  IconCalendar,
  IconCheck,
  IconClock,
  IconCopy,
  IconDotsVertical,
  IconDownload,
  IconFolderDown,
  IconInfinity,
  IconListDetails,
  IconPlayerPlay,
  IconPlayerStop,
  IconRefresh,
  IconShield,
  IconStar,
  IconTrash,
} from "@tabler/icons-react";

import type { MacroListItem } from "@/api";
import {
  STOPS,
  fmtAgo,
  fmtDur,
  repeatToIndex,
} from "@/format";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

const SPEEDS = ["0.25", "0.5", "1", "1.5", "2", "4"] as const;
const REPEAT_WORDS = [
  "Until I stop",
  "Once",
  "Twice",
  "3 times",
  "5 times",
  "10 times",
];

export interface MacroRowProps {
  macro: MacroListItem;
  guards: number;
  speed: string;
  active: boolean;
  favorite: boolean;
  selected: boolean;
  playing: boolean;
  iteration: number;
  totalReps: number;
  busy: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onFavorite: () => void;
  onRun: () => void;
  onStop: () => void;
  onRepeat: (index: number) => void;
  onDuplicate: () => void;
  onExport: () => void;
  onBundle: () => void;
  onDelete: () => void;
}

export function MacroRow(props: MacroRowProps) {
  const repeatIndex = repeatToIndex(
    props.macro.loop,
    props.macro.loop_count,
  );
  const lastPlayed = fmtAgo(props.macro.last_played ?? 0);

  return (
    <article
      className={cn(
        "macro-row group relative overflow-hidden rounded-xl border bg-card transition-[border-color,background-color,box-shadow]",
        props.active
          ? "border-primary shadow-[inset_3px_0_0_var(--primary),0_10px_28px_rgba(50,35,18,0.045)]"
          : "border-border hover:border-primary/45",
        props.playing && "bg-primary/[0.035]",
      )}
    >
      <button
        type="button"
        onClick={props.onToggle}
        className="absolute inset-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      >
        <span className="sr-only">Select {props.macro.name}</span>
      </button>

      <div className="macro-row-grid pointer-events-none relative grid items-center gap-4 px-4 py-4">
        <div className="flex min-w-0 items-center gap-4">
          <div className="flex size-12 shrink-0 items-center justify-center rounded-xl bg-primary/[0.08] text-primary">
            <IconListDetails className="size-6" strokeWidth={1.75} />
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h2 className="truncate text-[17px] font-semibold tracking-[-0.02em] text-foreground">
                {props.macro.name}
              </h2>
              {props.favorite && (
                <IconStar
                  className="size-[18px] shrink-0 fill-primary text-primary"
                  aria-label="Favorite"
                />
              )}
            </div>
            <div className="mt-2 flex flex-wrap gap-2">
              {!!props.macro.last_played && (
                <Badge className="rounded-md border-0 bg-success/10 text-success">
                  Recent
                </Badge>
              )}
              {props.macro.loop && (
                <Badge className="rounded-md border-0 bg-primary/10 text-primary">
                  Loop
                </Badge>
              )}
              {props.macro.category && (
                <Badge
                  variant="secondary"
                  className="rounded-md font-normal"
                >
                  {props.macro.category}
                </Badge>
              )}
              {props.guards > 0 && (
                <Badge
                  variant="outline"
                  className="rounded-md font-normal text-muted-foreground"
                >
                  <IconShield className="size-3" />
                  {props.guards}
                </Badge>
              )}
            </div>
          </div>
        </div>

        <dl className="grid grid-cols-2 gap-x-4 gap-y-3 text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <IconListDetails className="size-[17px]" strokeWidth={1.6} />
            <span>{props.macro.events.toLocaleString()} actions</span>
          </div>
          <div className="flex items-center gap-2">
            <IconClock className="size-[17px]" strokeWidth={1.6} />
            <span>{fmtDur(props.macro.duration)}</span>
          </div>
          <div className="col-span-2 flex items-center gap-2">
            <IconCalendar className="size-[17px]" strokeWidth={1.6} />
            <span>{lastPlayed ? `Last played ${lastPlayed}` : "Not played yet"}</span>
          </div>
        </dl>

        <div className="macro-row-actions pointer-events-auto flex items-center justify-end gap-2">
          <Select
            value={String(repeatIndex)}
            onValueChange={(value) => props.onRepeat(Number(value))}
          >
            <SelectTrigger
              aria-label={`Repeat ${props.macro.name}`}
              className="h-10 w-[126px] rounded-lg bg-background"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {STOPS.map((stop, index) => (
                <SelectItem key={stop} value={String(index)}>
                  Repeat: {stop}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          {props.playing ? (
            <Button
              variant="destructive"
              onClick={props.onStop}
              className="h-10 rounded-lg px-4"
            >
              <IconPlayerStop className="size-4 fill-current" />
              Stop
            </Button>
          ) : (
            <Button
              onClick={props.onRun}
              disabled={props.busy}
              className="h-10 rounded-lg px-4"
            >
              <IconPlayerPlay className="size-4 fill-current" />
              Run
            </Button>
          )}

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`More actions for ${props.macro.name}`}
                className="rounded-lg"
              >
                <IconDotsVertical className="size-[18px]" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-52">
              <DropdownMenuItem onSelect={props.onFavorite}>
                <IconStar className="size-4" />
                {props.favorite ? "Remove favorite" : "Add to favorites"}
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={props.onSelect}>
                <IconCheck className="size-4" />
                {props.selected ? "Deselect" : "Select"}
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={props.onDuplicate}>
                <IconCopy className="size-4" />
                Duplicate
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={props.onExport}>
                <IconDownload className="size-4" />
                Share as file
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={props.onBundle}>
                <IconFolderDown className="size-4" />
                Share with images
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={props.onDelete}
                className="text-destructive focus:text-destructive"
              >
                <IconTrash className="size-4" />
                Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {props.playing && (
        <div className="flex items-center gap-3 border-t border-border px-5 py-2.5">
          <Progress
            value={
              props.totalReps > 0
                ? (props.iteration / props.totalReps) * 100
                : undefined
            }
            className="h-1.5 flex-1"
          />
          <span className="text-xs font-medium text-primary">
            {props.totalReps > 0
              ? `Run ${props.iteration} of ${props.totalReps}`
              : `Run ${props.iteration}, continuous`}
          </span>
        </div>
      )}
    </article>
  );
}

export function MacroLibrarySummary({
  macros,
}: {
  macros: MacroListItem[];
}) {
  const totals = macros.reduce(
    (summary, macro) => ({
      actions: summary.actions + macro.events,
      duration: summary.duration + macro.duration,
      plays: summary.plays + macro.play_count,
      looping: summary.looping + Number(macro.loop),
    }),
    { actions: 0, duration: 0, plays: 0, looping: 0 },
  );

  const metrics = [
    ["Total actions", totals.actions.toLocaleString()],
    ["Combined duration", fmtDur(totals.duration)],
    ["Total plays", totals.plays.toLocaleString()],
    ["Looping", totals.looping.toLocaleString()],
  ] as const;

  return (
    <section
      aria-label="Library overview"
      className="mt-2 grid border-y border-border/70 bg-card/20 px-1 py-4 sm:grid-cols-[minmax(150px,0.9fr)_minmax(0,2.1fr)] sm:items-center"
    >
      <div className="px-3 pb-4 sm:pb-0">
        <p className="text-sm font-semibold text-foreground">Library overview</p>
        <p className="mt-1 text-xs text-muted-foreground">
          {macros.length.toLocaleString()} macro{macros.length === 1 ? "" : "s"} at a glance
        </p>
      </div>
      <dl className="grid grid-cols-2 sm:grid-cols-4">
        {metrics.map(([label, value], index) => (
          <div
            key={label}
            className={cn(
              "min-w-0 border-border/70 px-3 py-1",
              index > 0 && "sm:border-l",
              index % 2 === 1 && "border-l sm:border-l",
              index >= 2 && "mt-4 sm:mt-0",
            )}
          >
            <dt className="truncate text-[11px] font-medium text-muted-foreground">
              {label}
            </dt>
            <dd className="mt-1 truncate text-base font-semibold tabular-nums text-foreground">
              {value}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

interface MacroInspectorProps {
  macro: MacroListItem;
  favorite: boolean;
  busy: boolean;
  onFavorite: () => void;
  onRun: (repeat: number) => void;
  onDuplicate: () => void;
  onDelete: () => void;
}

export function MacroInspector(props: MacroInspectorProps) {
  const lastPlayed = fmtAgo(props.macro.last_played ?? 0);

  return (
    <aside
      aria-label={`Overview for ${props.macro.name}`}
      className="overflow-hidden rounded-xl border border-border bg-card shadow-[0_12px_34px_rgba(50,35,18,0.045)]"
    >
      <div className="p-4">
        <div className="flex items-start gap-4">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/[0.08] text-primary">
            <IconListDetails className="size-5" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="truncate text-xl font-semibold tracking-[-0.025em]">
                {props.macro.name}
              </h2>
              <button
                type="button"
                onClick={props.onFavorite}
                aria-label={
                  props.favorite ? "Remove from favorites" : "Add to favorites"
                }
                className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-primary"
              >
                <IconStar
                  className={cn(
                    "size-[18px]",
                    props.favorite && "fill-primary text-primary",
                  )}
                />
              </button>
            </div>
            <p className="mt-1 line-clamp-2 text-sm leading-5 text-muted-foreground">
              {props.macro.notes || "No description added yet."}
            </p>
          </div>
        </div>

        <div className="my-3 h-px bg-border" />

        <p className="mb-3 text-xs font-semibold text-foreground">Stats</p>
        <dl className="grid grid-cols-2 gap-x-4 gap-y-3">
          <Stat
            icon={IconListDetails}
            label="Total actions"
            value={props.macro.events.toLocaleString()}
          />
          <Stat
            icon={IconCalendar}
            label="Last played"
            value={lastPlayed || "Not yet"}
          />
          <Stat
            icon={IconClock}
            label="Duration"
            value={fmtDur(props.macro.duration)}
          />
          <Stat
            icon={IconRefresh}
            label="Times played"
            value={`${props.macro.play_count.toLocaleString()}x`}
          />
        </dl>

        <div className="my-3 h-px bg-border" />

        <p className="mb-3 text-xs font-semibold text-foreground">Quick run</p>
        <div className="grid grid-cols-3 gap-2">
          <QuickRun
            icon={IconPlayerPlay}
            label="Run once"
            disabled={props.busy}
            onClick={() => props.onRun(1)}
          />
          <QuickRun
            icon={IconRefresh}
            label="Run 5x"
            disabled={props.busy}
            onClick={() => props.onRun(5)}
          />
          <QuickRun
            icon={IconInfinity}
            label="Run continuously"
            iconOnly
            disabled={props.busy}
            onClick={() => props.onRun(0)}
          />
        </div>

        <div className="my-3 h-px bg-border" />

        <p className="mb-3 text-xs font-semibold text-foreground">More actions</p>
        <div className="grid grid-cols-2 gap-2">
          <Button
            variant="outline"
            onClick={props.onDuplicate}
            className="h-10 rounded-lg"
          >
            <IconCopy className="size-4" />
            Duplicate
          </Button>
          <Button
            variant="outline"
            onClick={props.onDelete}
            className="h-10 rounded-lg text-destructive hover:bg-destructive/10 hover:text-destructive"
          >
            <IconTrash className="size-4" />
            Delete
          </Button>
        </div>
      </div>
    </aside>
  );
}

interface MacroEditorPanelProps {
  macro: MacroListItem;
  speed: string;
  focusName: boolean;
  playing: boolean;
  onStop: () => void;
  onSpeed: (value: string) => void;
  onRepeat: (index: number) => void;
  onRename: (value: string) => void;
  onCategory: (value: string) => void;
  onNotes: (value: string) => void;
  onSavePreset: () => void;
  onExport: () => void;
  onBundle: () => void;
}

export function MacroEditorPanel(props: MacroEditorPanelProps) {
  const nameRef = useRef<HTMLInputElement>(null);
  const [draftName, setDraftName] = useState(props.macro.name);
  const [draftCategory, setDraftCategory] = useState(
    props.macro.category ?? "",
  );
  const [draftNotes, setDraftNotes] = useState(props.macro.notes ?? "");

  useEffect(() => setDraftName(props.macro.name), [props.macro.name]);
  useEffect(
    () => setDraftCategory(props.macro.category ?? ""),
    [props.macro.category],
  );
  useEffect(
    () => setDraftNotes(props.macro.notes ?? ""),
    [props.macro.notes],
  );
  useEffect(() => {
    if (!props.focusName) return;
    nameRef.current?.focus();
    nameRef.current?.select();
  }, [props.focusName, props.macro.name]);

  const repeatIndex = repeatToIndex(
    props.macro.loop,
    props.macro.loop_count,
  );
  const commitName = () => {
    const value = draftName.trim();
    if (!value || value === props.macro.name) {
      setDraftName(props.macro.name);
      return;
    }
    props.onRename(value);
  };
  const commitCategory = () => {
    const value = draftCategory.trim();
    if (value !== (props.macro.category ?? "")) props.onCategory(value);
  };
  const commitNotes = () => {
    const value = draftNotes.trim();
    if (value !== (props.macro.notes ?? "")) props.onNotes(value);
  };

  return (
    <aside
      aria-label={`Edit ${props.macro.name}`}
      className="shrink-0 overflow-hidden rounded-xl border border-border bg-card shadow-[0_12px_34px_rgba(50,35,18,0.045)]"
    >
      <div className="border-b border-border px-5 py-4">
        <p className="text-sm font-semibold text-foreground">Edit macro</p>
        <p className="mt-0.5 truncate text-xs text-muted-foreground">
          {props.macro.name}
        </p>
      </div>
      <div className="p-5">
        <div className="grid gap-4">
          <InspectorField label="Name">
            <Input
              ref={nameRef}
              aria-label="Name"
              value={draftName}
              onChange={(event) => setDraftName(event.target.value)}
              onBlur={commitName}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
                if (event.key === "Escape") {
                  setDraftName(props.macro.name);
                }
              }}
            />
          </InspectorField>
          <InspectorField label="Category">
            <Input
              aria-label="Category"
              value={draftCategory}
              onChange={(event) => setDraftCategory(event.target.value)}
              onBlur={commitCategory}
              onKeyDown={(event) =>
                event.key === "Enter" && event.currentTarget.blur()
              }
              placeholder="For example, Farming"
            />
          </InspectorField>
          <InspectorField label="Notes">
            <Textarea
              aria-label="Notes"
              rows={3}
              value={draftNotes}
              onChange={(event) => setDraftNotes(event.target.value)}
              onBlur={commitNotes}
              placeholder="What this macro does"
            />
          </InspectorField>

          <InspectorField label="Playback speed">
            <div className="grid grid-cols-6 gap-1.5">
              {SPEEDS.map((speed) => (
                <Choice
                  key={speed}
                  active={props.speed === speed}
                  onClick={() => props.onSpeed(speed)}
                >
                  {speed}x
                </Choice>
              ))}
            </div>
          </InspectorField>

          <InspectorField label="Repeat">
            <Select
              value={String(repeatIndex)}
              onValueChange={(value) => props.onRepeat(Number(value))}
            >
              <SelectTrigger aria-label="Edit repeat">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STOPS.map((stop, index) => (
                  <SelectItem key={stop} value={String(index)}>
                    {REPEAT_WORDS[index]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
        </div>

        <div className="mt-4 grid grid-cols-2 gap-2 border-t border-border pt-3">
          <Button variant="ghost" size="sm" onClick={props.onSavePreset}>
            <IconBookmarkPlus className="size-4" />
            Save preset
          </Button>
          <Button variant="ghost" size="sm" onClick={props.onExport}>
            <IconDownload className="size-4" />
            Share file
          </Button>
          <Button variant="ghost" size="sm" onClick={props.onBundle}>
            <IconFolderDown className="size-4" />
            Share images
          </Button>
          {props.playing && (
            <Button variant="destructive" size="sm" onClick={props.onStop}>
              <IconPlayerStop className="size-4" />
              Stop playback
            </Button>
          )}
        </div>
      </div>
    </aside>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof IconClock;
  label: string;
  value: string;
}) {
  return (
    <div className="flex min-w-0 gap-2.5">
      <Icon className="mt-0.5 size-[18px] shrink-0 text-muted-foreground" strokeWidth={1.65} />
      <div className="min-w-0">
        <dt className="text-xs text-muted-foreground">{label}</dt>
        <dd className="mt-0.5 truncate text-sm font-semibold text-foreground">
          {value}
        </dd>
      </div>
    </div>
  );
}

function QuickRun({
  icon: Icon,
  label,
  iconOnly = false,
  disabled,
  onClick,
}: {
  icon: typeof IconClock;
  label: string;
  iconOnly?: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      aria-label={label}
      className="flex min-w-0 items-center justify-center rounded-lg border border-border px-2 py-2.5 text-center transition-colors hover:border-primary/45 hover:bg-primary/[0.035] disabled:pointer-events-none disabled:opacity-50"
    >
      <Icon
        className={cn(iconOnly ? "size-6" : "size-[18px]")}
        strokeWidth={1.6}
      />
      {!iconOnly && (
        <span className="ml-1.5 whitespace-nowrap text-xs font-medium text-foreground">
          {label}
        </span>
      )}
    </button>
  );
}

function InspectorField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-1.5">
      <span className="text-xs font-medium text-foreground">{label}</span>
      {children}
    </div>
  );
}

function Choice({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-md border px-1 py-1.5 text-[11px] font-medium transition-colors",
        active
          ? "border-primary bg-primary/10 text-primary"
          : "border-border text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}
