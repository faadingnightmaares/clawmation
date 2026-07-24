import { useEffect, useState } from "react";

import { getStatus, type Status } from "./api";

/** Heartbeat interval, matching the Python UI's 500ms `setInterval`. */
const POLL_MS = 500;

/**
 * Polls `get_status` once immediately and then every 500ms, returning the
 * latest payload (or `null` before the first response). Mirrors the Python
 * UI's `onTick`: a single failed poll is best-effort — it skips this tick and
 * retries on the next one rather than tearing down the UI.
 */
export function useStatus(): Status | null {
  const [status, setStatus] = useState<Status | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const next = await getStatus();
        if (alive) setStatus(next);
      } catch {
        // Transient poll failure (e.g. backend mid-startup) — retry next tick.
      }
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  return status;
}
