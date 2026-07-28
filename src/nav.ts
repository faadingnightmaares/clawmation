import type { ComponentType } from "react";
import {
  HomeSimple,
  MediaVideoList,
  Eye,
  Repeat,
  Settings,
} from "iconoir-react";

export type NavIcon = ComponentType<{
  className?: string;
  strokeWidth?: number;
}>;

// The routing contract every view and handler switches on. NOTE the "Watch"
// surface's id is `vision`, a historical name kept for fidelity with the Rust
// side, where the whole feature is `commands::vision`.
export type ViewId =
  "dashboard" | "macros" | "nodes" | "vision" | "settings";

export interface NavMeta {
  id: ViewId;
  /** What the user sees: plain words, not the internal id. */
  label: string;
  Icon: NavIcon;
  /** `switch` rides the command-bar switcher; `utility` is an icon beside it. */
  group: "switch" | "utility";
  /** Disabled surfaces stay visible so upcoming work is discoverable. */
  disabled?: boolean;
  badge?: string;
}

export const VIEW_ICON_STROKE_WIDTH = 1.8;

export const VIEW_ICONS: Record<ViewId, NavIcon> = {
  dashboard: HomeSimple,
  macros: MediaVideoList,
  vision: Eye,
  nodes: Repeat,
  settings: Settings,
};

// Order also defines the Alt+1..6 shortcuts (index + 1).
//
// Loops is its own workspace because the graph needs the full app viewport.
// Settings remains a separate utility rather than a working surface.
export const NAV: NavMeta[] = [
  {
    id: "dashboard",
    label: "Home",
    Icon: VIEW_ICONS.dashboard,
    group: "switch",
  },
  {
    id: "macros",
    label: "Macros",
    Icon: VIEW_ICONS.macros,
    group: "switch",
  },
  {
    id: "vision",
    label: "Watch",
    Icon: VIEW_ICONS.vision,
    group: "switch",
  },
  {
    id: "nodes",
    label: "Loops",
    Icon: VIEW_ICONS.nodes,
    group: "switch",
  },
  {
    id: "settings",
    label: "Settings",
    Icon: VIEW_ICONS.settings,
    group: "utility",
  },
];

export const PRIMARY_VIEWS = NAV.filter((n) => n.group === "switch");
