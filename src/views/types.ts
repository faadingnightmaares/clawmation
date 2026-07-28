import type { Status } from "@/api";
import type { ViewId } from "@/nav";

export interface ViewProps {
  status: Status | null;
  navigate: (v: ViewId) => void;
  active?: boolean;
}
