import {
  SquaresFour,
  List,
  ShieldCheck,
  Eye,
  FlowArrow,
  BookOpenText,
  Gear,
  type Icon,
} from "@phosphor-icons/react";

export type ViewId =
  | "dashboard"
  | "macros"
  | "ai"
  | "vision"
  | "chains"
  | "guide"
  | "settings";

export interface NavItem {
  id: ViewId;
  label: string;
  Icon: Icon;
}

// Sidebar order also defines the Alt+1..7 shortcuts (index + 1). NOTE: the
// "Guards" item's internal view id is `ai` — a historical name kept for
// fidelity, not `guards`.
export const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Dashboard", Icon: SquaresFour },
  { id: "macros", label: "Macros", Icon: List },
  { id: "ai", label: "Guards", Icon: ShieldCheck },
  { id: "vision", label: "Vision", Icon: Eye },
  { id: "chains", label: "Chains", Icon: FlowArrow },
  { id: "guide", label: "Guide", Icon: BookOpenText },
  { id: "settings", label: "Settings", Icon: Gear },
];
