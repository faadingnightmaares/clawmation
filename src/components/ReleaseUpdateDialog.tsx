import { useMemo } from "react";
import { ArrowRight, Download, Sparkles } from "lucide-react";

import type { UpdateInfo } from "@/api";
import { parseReleaseNotes, type ReleaseSectionContent } from "@/lib/releaseNotes";
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
import { Progress } from "@/components/ui/progress";

interface ReleaseUpdateDialogProps {
  info: UpdateInfo | null;
  installing: boolean;
  progress: number | null;
  onDismiss: () => void;
  onInstall: () => void;
}

export function ReleaseUpdateDialog({
  info,
  installing,
  progress,
  onDismiss,
  onInstall,
}: ReleaseUpdateDialogProps) {
  const parsed = useMemo(() => parseReleaseNotes(info?.notes), [info?.notes]);
  if (!info) return null;

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !installing) onDismiss();
      }}
    >
      <AlertDialogContent className="max-h-[calc(100vh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-xl">
        <AlertDialogHeader className="gap-0 border-b border-border bg-card px-6 py-5">
          <div className="mb-3 flex items-center gap-2 text-[0.6875rem] font-semibold tracking-[0.14em] text-primary uppercase">
            <span className="flex size-6 items-center justify-center rounded-md bg-primary/10">
              <Sparkles className="size-3.5" aria-hidden="true" />
            </span>
            Update available
          </div>
          <AlertDialogTitle>Clawmation {info.latest} is ready</AlertDialogTitle>
          <AlertDialogDescription className="mt-1.5 max-w-lg leading-relaxed">
            {parsed.summary}
          </AlertDialogDescription>

          <div
            className="mt-4 flex w-fit items-center gap-2 rounded-lg border border-border bg-secondary/45 px-3 py-2 font-mono text-xs"
            aria-label={`Version ${info.current} to ${info.latest}`}
          >
            <span className="text-muted-foreground">{info.current}</span>
            <ArrowRight className="size-3.5 text-primary" aria-hidden="true" />
            <span data-version="available" className="font-medium text-foreground">
              {info.latest}
            </span>
          </div>
        </AlertDialogHeader>

        {installing ? (
          <div className="flex min-h-48 flex-col justify-center gap-4 px-6 py-8">
            <span className="flex size-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <Download className="size-5" aria-hidden="true" />
            </span>
            <div className="space-y-2.5">
              <Progress value={progress ?? 0} aria-label="Update download progress" />
              <p className="text-sm font-medium text-foreground" aria-live="polite">
                {progress === null ? "Downloading…" : `Downloading… ${progress}%`}
              </p>
              <p className="text-xs leading-relaxed text-muted-foreground">
                Keep Clawmation open. It will restart automatically when the update is ready.
              </p>
            </div>
          </div>
        ) : (
          <div
            role="region"
            aria-label="Release highlights"
            tabIndex={0}
            className="min-h-0 overflow-y-auto overscroll-contain px-6 py-5 break-words [overflow-wrap:anywhere] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          >
            <p className="mb-4 text-[0.6875rem] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
              Release highlights
            </p>
            <div className="space-y-5">
              {parsed.sections.map((section, sectionIndex) => (
                <section
                  key={`${section.heading ?? "notes"}-${sectionIndex}`}
                  className="space-y-3"
                >
                  {section.heading && (
                    <h3 className="text-sm font-semibold text-foreground">{section.heading}</h3>
                  )}
                  {section.content.map((content, contentIndex) => (
                    <ReleaseContent
                      key={`${content.kind}-${contentIndex}`}
                      content={content}
                    />
                  ))}
                </section>
              ))}
            </div>
          </div>
        )}

        {!installing && (
          <AlertDialogFooter className="items-stretch border-t border-border bg-card px-6 py-4 sm:items-center">
            <p className="text-left text-xs leading-relaxed text-muted-foreground sm:mr-auto sm:max-w-52">
              Installing restarts the app. Finish any active run first.
            </p>
            <AlertDialogCancel>Not now</AlertDialogCancel>
            <AlertDialogAction
              onClick={(event) => {
                event.preventDefault();
                onInstall();
              }}
            >
              <Download className="size-4" aria-hidden="true" />
              Install and restart
            </AlertDialogAction>
          </AlertDialogFooter>
        )}
      </AlertDialogContent>
    </AlertDialog>
  );
}

function ReleaseContent({ content }: { content: ReleaseSectionContent }) {
  if (content.kind === "paragraph") {
    return <p className="text-sm leading-6 text-muted-foreground">{content.text}</p>;
  }

  return (
    <ol className="space-y-2.5">
      {content.items.map((item) => (
        <li
          key={`${item.number}-${item.title}`}
          className="grid grid-cols-[2rem_minmax(0,1fr)] gap-3 rounded-xl border border-border bg-secondary/25 p-3.5"
        >
          <span
            aria-hidden="true"
            className="flex size-8 items-center justify-center rounded-lg bg-primary/10 font-mono text-[0.6875rem] font-medium text-primary"
          >
            {String(item.number).padStart(2, "0")}
          </span>
          <div className="min-w-0 pt-0.5">
            <p className="text-sm font-medium leading-5 text-foreground">{item.title}</p>
            {item.detail && (
              <p className="mt-1 text-xs leading-5 text-muted-foreground">{item.detail}</p>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
