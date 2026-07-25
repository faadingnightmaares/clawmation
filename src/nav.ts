import {
  House,
  ListVideo,
  ShieldCheck,
  Eye,
  Workflow,
  BookOpen,
  Settings,
  type LucideIcon,
} from "lucide-react";

// The routing contract kept from the backend era: these ids are what every view
// and handler switch on. NOTE the "Guards" surface's id is `ai` and the "Watch"
// surface's id is `vision` — historical names kept for fidelity with the Rust side.
export type ViewId =
  | "dashboard"
  | "macros"
  | "ai"
  | "vision"
  | "chains"
  | "guide"
  | "settings";

export interface NavMeta {
  id: ViewId;
  /** What the user sees — plain words, not the internal id. */
  label: string;
  Icon: LucideIcon;
  /** `primary` rides the command-bar switcher; `more` lives in the More menu. */
  group: "primary" | "more";
}

// Order also defines the Alt+1..7 shortcuts (index + 1), unchanged from before so
// muscle memory carries over. Two primary surfaces (Macros, Watch) sit in the bar;
// the rest fold into More so the very first click isn't a wall of seven nouns.
export const NAV: NavMeta[] = [
  { id: "dashboard", label: "Home", Icon: House, group: "more" },
  { id: "macros", label: "Macros", Icon: ListVideo, group: "primary" },
  { id: "ai", label: "Guards", Icon: ShieldCheck, group: "more" },
  { id: "vision", label: "Watch", Icon: Eye, group: "primary" },
  { id: "chains", label: "Chains", Icon: Workflow, group: "more" },
  { id: "guide", label: "Guide", Icon: BookOpen, group: "more" },
  { id: "settings", label: "Settings", Icon: Settings, group: "more" },
];

export const PRIMARY_VIEWS = NAV.filter((n) => n.group === "primary");
export const MORE_VIEWS = NAV.filter((n) => n.group === "more");

export const NAV_BY_ID = Object.fromEntries(
  NAV.map((n) => [n.id, n]),
) as Record<ViewId, NavMeta>;
