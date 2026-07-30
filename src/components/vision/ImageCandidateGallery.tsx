import {
  ImageSquare,
  MagicWand,
  Plus,
  SpinnerGap,
  UploadSimple,
  X,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { MAX_IMAGE_CANDIDATES } from "@/lib/visionImages";

interface ImageCandidateGalleryProps {
  candidates: readonly string[];
  thumbnails?: readonly (string | null | undefined)[];
  busy?: boolean;
  dragOver?: boolean;
  magicLabel?: string;
  onChoose: () => void;
  onMagicSelect: () => void;
  onRemove: (index: number) => void;
  onDrop: (file: File) => void;
  onDragOverChange?: (over: boolean) => void;
}

const fileName = (path: string) => path.split(/[\\/]/).pop() || "Image";

export function ImageCandidateGallery({
  candidates,
  thumbnails = [],
  busy = false,
  dragOver = false,
  magicLabel = "Magic select from screen",
  onChoose,
  onMagicSelect,
  onRemove,
  onDrop,
  onDragOverChange,
}: ImageCandidateGalleryProps) {
  const full = candidates.length >= MAX_IMAGE_CANDIDATES;
  return (
    <div
      className={cn(
        "grid gap-2 rounded-lg border border-dashed p-2.5 transition-colors",
        dragOver
          ? "border-primary bg-primary/[0.06]"
          : "border-border bg-background",
      )}
      onDragEnter={(event) => {
        event.preventDefault();
        onDragOverChange?.(true);
      }}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = full ? "none" : "copy";
        onDragOverChange?.(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          onDragOverChange?.(false);
        }
      }}
      onDrop={(event) => {
        event.preventDefault();
        onDragOverChange?.(false);
        const file = event.dataTransfer.files[0];
        if (file && !full) onDrop(file);
      }}
    >
      {candidates.length > 0 ? (
        <div className="grid grid-cols-2 gap-2">
          {candidates.map((path, index) => (
            <div
              key={`${path}-${index}`}
              className="group relative min-w-0 overflow-hidden rounded-md border border-border bg-card"
            >
              <div className="grid h-20 place-items-center overflow-hidden bg-muted/25">
                {thumbnails[index] ? (
                  <img
                    src={thumbnails[index] ?? ""}
                    alt=""
                    className="size-full object-contain"
                  />
                ) : (
                  <ImageSquare
                    className="size-7 text-muted-foreground/60"
                    weight="duotone"
                  />
                )}
              </div>
              <div className="min-w-0 border-t border-border px-2 py-1.5 pr-7">
                <p
                  className="truncate text-[10px] font-medium text-foreground"
                  title={path}
                >
                  {fileName(path)}
                </p>
                <p className="mt-0.5 text-[9px] text-muted-foreground">
                  {index === 0 ? "Primary" : `Alternative ${index}`}
                </p>
              </div>
              <button
                type="button"
                aria-label={`Remove ${fileName(path)}`}
                className="absolute bottom-1.5 right-1.5 grid size-5 place-items-center rounded text-muted-foreground opacity-70 transition-colors hover:bg-destructive/10 hover:text-destructive focus-visible:opacity-100 group-hover:opacity-100"
                onClick={() => onRemove(index)}
                disabled={busy}
              >
                <X className="size-3.5" />
              </button>
            </div>
          ))}
          {!full && (
            <button
              type="button"
              className="grid min-h-[112px] place-items-center rounded-md border border-dashed border-border bg-muted/15 text-center text-muted-foreground transition-colors hover:border-primary/55 hover:bg-primary/[0.04] hover:text-foreground"
              onClick={onChoose}
              disabled={busy}
            >
              <span>
                <Plus className="mx-auto size-5 text-primary" weight="bold" />
                <span className="mt-1.5 block text-[10px] font-medium">Add another state</span>
              </span>
            </button>
          )}
        </div>
      ) : (
        <button
          type="button"
          aria-label="Choose image"
          className="grid min-h-32 place-items-center rounded-md bg-muted/15 px-4 text-center transition-colors hover:bg-muted/30"
          onClick={onChoose}
          disabled={busy}
        >
          {busy ? (
            <span>
              <SpinnerGap className="mx-auto size-6 animate-spin text-primary" />
              <span className="mt-2 block text-xs text-muted-foreground">
                Preparing image…
              </span>
            </span>
          ) : (
            <span>
              <UploadSimple className="mx-auto size-7 text-primary" weight="duotone" />
              <span className="mt-2 block text-xs font-medium text-foreground">
                Drag and drop an image
              </span>
              <span className="mt-1 block text-[10px] text-muted-foreground">
                or click to choose one
              </span>
            </span>
          )}
        </button>
      )}

      <div className="flex items-center justify-between gap-2">
        <span className="text-[10px] tabular-nums text-muted-foreground">
          {candidates.length} / {MAX_IMAGE_CANDIDATES} images
        </span>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-8"
          onClick={onMagicSelect}
          disabled={busy || full}
        >
          {busy ? (
            <SpinnerGap className="size-4 animate-spin" />
          ) : (
            <MagicWand className="size-4" weight="duotone" />
          )}
          {magicLabel}
        </Button>
      </div>
    </div>
  );
}
