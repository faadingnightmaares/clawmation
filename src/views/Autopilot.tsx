import { useState } from "react";
import { ShieldCheck, Workflow } from "lucide-react";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Guards } from "./Guards";
import { Chains } from "./Chains";
import type { ViewProps } from "./types";

/**
 * Guards and Chains under one roof. They were separate pages, which made them
 * look like separate ideas — they are both answers to "keep running while I'm
 * not here", and you reach for them in the same breath.
 *
 * The inactive tab stays unmounted (Radix's default), which matters here: the
 * Chains panel runs a 500ms poll for the live-run banner and must not keep it up
 * while you are reading the Guards list.
 */
export function Autopilot(props: ViewProps) {
  const [tab, setTab] = useState("guards");

  return (
    <div className="space-y-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Autopilot</h1>
        <p className="text-sm text-muted-foreground">
          Keep your macros running while you're away — safety nets that catch trouble, and chains
          that play them one after another.
        </p>
      </header>

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList className="w-full sm:w-auto">
          <TabsTrigger value="guards" className="sm:px-6">
            <ShieldCheck />
            Guards
          </TabsTrigger>
          <TabsTrigger value="chains" className="sm:px-6">
            <Workflow />
            Chains
          </TabsTrigger>
        </TabsList>

        <TabsContent value="guards" className="pt-4">
          <Guards {...props} />
        </TabsContent>
        <TabsContent value="chains" className="pt-4">
          <Chains {...props} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
