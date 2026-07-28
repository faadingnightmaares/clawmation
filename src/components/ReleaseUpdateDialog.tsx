import { useMemo } from "react";
import {
  ArrowRight,
  CheckCircle,
  DownloadSimple,
  Sparkle,
} from "@phosphor-icons/react";

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
      <AlertDialogContent className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-2xl">
        <AlertDialogHeader className="gap-0 border-b border-border bg-card px-6 py-6 sm:px-8">
          <div className="mb-3 flex items-center gap-2 text-[0.6875rem] font-semibold tracking-[0.14em] text-primary uppercase">
            <Sparkle className="size-4" weight="fill" aria-hidden="true" />
            Update available
          </div>
          <AlertDialogTitle className="text-xl tracking-[-0.025em]">
            Clawmation {info.latest} is ready
          </AlertDialogTitle>
          <AlertDialogDescription className="mt-2 max-w-xl leading-6">
            {parsed.summary}
          </AlertDialogDescription>

          <div
            className="mt-4 flex items-center gap-2 font-mono text-[11px]"
            aria-label={`Version ${info.current} to ${info.latest}`}
          >
            <span className="text-muted-foreground">{info.current}</span>
            <ArrowRight className="size-3.5 text-primary" weight="bold" aria-hidden="true" />
            <span data-version="available" className="font-medium text-foreground">
              {info.latest}
            </span>
          </div>
        </AlertDialogHeader>

        {installing ? (
          <div className="flex min-h-52 flex-col justify-center gap-4 px-6 py-9 sm:px-8">
            <span className="flex size-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <DownloadSimple className="size-5" weight="bold" aria-hidden="true" />
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
            className="min-h-0 overflow-y-auto overscroll-contain px-6 py-6 break-words [overflow-wrap:anywhere] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:px-8"
          >
            <div className="relative space-y-7 before:absolute before:top-2 before:bottom-2 before:left-[5px] before:w-px before:bg-border">
              {parsed.sections.map((section, sectionIndex) => (
                <section
                  key={`${section.heading ?? "notes"}-${sectionIndex}`}
                  className="relative space-y-3 pl-7"
                >
                  {section.heading && (
                    <h3 className="relative text-sm font-semibold text-foreground">
                      <span
                        aria-hidden="true"
                        className="absolute top-[0.4rem] -left-[1.72rem] size-[11px] rounded-full border-[3px] border-card bg-primary"
                      />
                      {section.heading}
                    </h3>
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
          <AlertDialogFooter className="items-stretch border-t border-border bg-card px-6 py-4 sm:items-center sm:px-8">
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
              <DownloadSimple className="size-4" weight="bold" aria-hidden="true" />
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
    <ol className="divide-y divide-border border-y border-border">
      {content.items.map((item) => (
        <li
          key={`${item.number}-${item.title}`}
          className="grid grid-cols-[1.25rem_minmax(0,1fr)] gap-3 py-3.5"
        >
          <CheckCircle
            className="mt-0.5 size-4 text-primary"
            weight="fill"
            aria-hidden="true"
          />
          <div className="min-w-0">
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
