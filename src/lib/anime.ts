import { useEffect, useRef } from "react";
import { animate, stagger } from "animejs";

/** True when the OS asks for less motion — every animation here must have a
 *  still fallback that lands on the same final state. */
export const reducedMotion = () =>
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Stagger-fade a container's direct children in on mount and whenever `key`
 * changes (e.g. a list re-filters). Returns a ref to spread onto the container.
 * No-ops under prefers-reduced-motion, leaving children at full opacity.
 */
export function useStaggerIn<T extends HTMLElement>(key?: unknown) {
  const ref = useRef<T>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el || el.children.length === 0) return;
    const kids = Array.from(el.children) as HTMLElement[];
    if (reducedMotion()) {
      kids.forEach((k) => (k.style.opacity = "1"));
      return;
    }
    animate(kids, {
      opacity: [0, 1],
      translateY: [10, 0],
      duration: 440,
      delay: stagger(45),
      ease: "out(3)",
    });
  }, [key]);
  return ref;
}
