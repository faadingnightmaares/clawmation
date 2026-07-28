import { useCallback, useRef, useState } from "react";

interface HistoryControls {
  checkpoint: () => void;
  undo: () => void;
  redo: () => void;
  reset: () => void;
  canUndo: boolean;
  canRedo: boolean;
}

/**
 * Small immutable-snapshot history for the node editor. Graph updates already
 * replace arrays and changed nodes, so snapshots can share untouched data
 * safely instead of cloning large embedded macro payloads on every edit.
 */
export function useNodeGraphHistory<T>(
  current: T,
  restore: (snapshot: T) => void,
  limit = 60,
): HistoryControls {
  const currentRef = useRef(current);
  const restoreRef = useRef(restore);
  const pastRef = useRef<T[]>([]);
  const futureRef = useRef<T[]>([]);
  const [, refresh] = useState(0);
  currentRef.current = current;
  restoreRef.current = restore;

  const updateControls = useCallback(() => refresh((value) => value + 1), []);

  const checkpoint = useCallback(() => {
    pastRef.current.push(currentRef.current);
    if (pastRef.current.length > limit) pastRef.current.shift();
    futureRef.current = [];
    updateControls();
  }, [limit, updateControls]);

  const undo = useCallback(() => {
    const previous = pastRef.current.pop();
    if (!previous) return;
    futureRef.current.push(currentRef.current);
    restoreRef.current(previous);
    updateControls();
  }, [updateControls]);

  const redo = useCallback(() => {
    const next = futureRef.current.pop();
    if (!next) return;
    pastRef.current.push(currentRef.current);
    restoreRef.current(next);
    updateControls();
  }, [updateControls]);

  const reset = useCallback(() => {
    pastRef.current = [];
    futureRef.current = [];
    updateControls();
  }, [updateControls]);

  return {
    checkpoint,
    undo,
    redo,
    reset,
    canUndo: pastRef.current.length > 0,
    canRedo: futureRef.current.length > 0,
  };
}
