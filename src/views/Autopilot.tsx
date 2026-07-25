import { Chains } from "./Chains";
import { Guards } from "./Guards";
import type { ViewProps } from "./types";

/**
 * Guards and chains, on one page. They used to be two tabs, which made them look
 * like two ideas you had to choose between. They are both answers to "keep
 * running while I'm not here", and you set them up in the same sitting. Chains
 * come first because they are the thing you start; guards are what protect it.
 *
 * Both halves are mounted the whole time, so the Chains poll now backs off while
 * nothing is running rather than relying on being unmounted.
 */
export function Autopilot(props: ViewProps) {
  return (
    <div className="flex flex-col gap-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Autopilot</h1>
        <p className="text-sm text-muted-foreground">
          Keep your macros running while you're away, with chains that play them one after another, and
          guards that catch trouble mid-run.
        </p>
      </header>

      <Chains {...props} />
      <Guards {...props} />
    </div>
  );
}
