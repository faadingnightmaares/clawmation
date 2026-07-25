import { cn } from "@/lib/utils";

/** The cat mark + wordmark, the one ownable signature. Serif retired: the
 *  wordmark is Inter now, set tight. */
export function Logo({ className }: { className?: string }) {
  return (
    <span className={cn("flex items-center gap-2 select-none", className)}>
      <img
        src="/Clawmation.svg"
        alt=""
        aria-hidden="true"
        className="h-[26px] w-auto"
      />
      <span className="text-[15px] font-semibold tracking-tight text-foreground">
        Clawmation
      </span>
    </span>
  );
}
