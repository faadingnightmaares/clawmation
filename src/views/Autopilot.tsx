import { useState } from "react";
import { Shield, Workflow } from "lucide-react";

import { SplitView } from "@/components/SplitView";
import { Chains } from "./Chains";
import { Guards } from "./Guards";
import type { ViewProps } from "./types";

/**
 * Guards and chains, one page with a rail of its own. Chains come first
 * because they are the thing you start; protection is what keeps a run safe
 * once it is going. Both halves stay mounted behind the rail, so switching
 * to the other one doesn't pause the Chains poll or drop anything mid-edit.
 */
export function Autopilot(props: ViewProps) {
  const [pane, setPane] = useState<"chains" | "protection">("chains");
  return (
    <div className="flex flex-col gap-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Autopilot</h1>
        <p className="text-sm text-muted-foreground">
          Keep your macros running while you're away, with chains that play them one after another, and
          guards that catch trouble mid-run.
        </p>
      </header>

      <SplitView
        label="Autopilot sections"
        items={[
          { id: "chains", label: "Chains", Icon: Workflow },
          { id: "protection", label: "Protection", Icon: Shield },
        ]}
        active={pane}
        onSelect={(id) => setPane(id)}
      >
        <div className={pane === "chains" ? undefined : "hidden"}>
          <Chains {...props} />
        </div>
        <div className={pane === "protection" ? undefined : "hidden"}>
          <Guards {...props} />
        </div>
      </SplitView>
    </div>
  );
}
