import { House, ListVideo, Eye, Workflow, Settings, type LucideIcon } from "lucide-react";

// The routing contract every view and handler switches on. NOTE the "Watch"
// surface's id is `vision` — a historical name kept for fidelity with the Rust
// side, where the whole feature is `commands::vision`.
export type ViewId = "dashboard" | "macros" | "vision" | "autopilot" | "settings";

export interface NavMeta {
  id: ViewId;
  /** What the user sees — plain words, not the internal id. */
  label: string;
  Icon: LucideIcon;
  /** `switch` rides the command-bar switcher; `utility` is an icon beside it. */
  group: "switch" | "utility";
}

// Order also defines the Alt+1..5 shortcuts (index + 1).
//
// Four surfaces, not seven: Guards and Chains are two halves of "run it without
// me" and now share Autopilot, and the Guide reads as reference material, so it
// sits inside Settings. That leaves few enough that every surface fits in the bar
// and nothing hides behind a More menu — the whole app is one click deep.
export const NAV: NavMeta[] = [
  { id: "dashboard", label: "Home", Icon: House, group: "switch" },
  { id: "macros", label: "Macros", Icon: ListVideo, group: "switch" },
  { id: "vision", label: "Watch", Icon: Eye, group: "switch" },
  { id: "autopilot", label: "Autopilot", Icon: Workflow, group: "switch" },
  { id: "settings", label: "Settings", Icon: Settings, group: "utility" },
];

export const PRIMARY_VIEWS = NAV.filter((n) => n.group === "switch");
