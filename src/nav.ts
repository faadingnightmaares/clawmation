import {
  House,
  ListVideo,
  GitBranch,
  Eye,
  Workflow,
  Settings,
  type LucideIcon,
} from "lucide-react";

// The routing contract every view and handler switches on. NOTE the "Watch"
// surface's id is `vision`, a historical name kept for fidelity with the Rust
// side, where the whole feature is `commands::vision`.
export type ViewId =
  "dashboard" | "macros" | "nodes" | "vision" | "autopilot" | "settings";

export interface NavMeta {
  id: ViewId;
  /** What the user sees: plain words, not the internal id. */
  label: string;
  Icon: LucideIcon;
  /** `switch` rides the command-bar switcher; `utility` is an icon beside it. */
  group: "switch" | "utility";
  /** Disabled surfaces stay visible so upcoming work is discoverable. */
  disabled?: boolean;
  badge?: string;
}

// Order also defines the Alt+1..6 shortcuts (index + 1).
//
// Nodes is its own workspace because the graph needs the full app viewport.
// Settings remains a separate utility rather than a working surface.
export const NAV: NavMeta[] = [
  { id: "dashboard", label: "Home", Icon: House, group: "switch" },
  { id: "macros", label: "Macros", Icon: ListVideo, group: "switch" },
  { id: "vision", label: "Watch", Icon: Eye, group: "switch" },
  { id: "autopilot", label: "Autopilot", Icon: Workflow, group: "switch" },
  {
    id: "nodes",
    label: "Nodes",
    Icon: GitBranch,
    group: "switch",
    disabled: true,
    badge: "Soon",
  },
  { id: "settings", label: "Settings", Icon: Settings, group: "utility" },
];

export const PRIMARY_VIEWS = NAV.filter((n) => n.group === "switch");
