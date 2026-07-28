import { useLayoutEffect, useRef } from "react";
import { animate } from "animejs";

/** True when the OS asks for less motion. Every animation here must have a
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
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || el.children.length === 0) return;
    const kids = Array.from(el.children) as HTMLElement[];
    if (reducedMotion()) {
      kids.forEach((kid) => {
        kid.style.removeProperty("opacity");
        kid.style.removeProperty("transform");
      });
      return;
    }

    // Layout effects run before paint, so users never see the unanimated list
    // for one frame. Keep the travel tiny and cap the stagger: a long macro list
    // should settle as quickly as a short one.
    const animation = animate(kids, {
      opacity: [0, 1],
      translateY: [3, 0],
      duration: 190,
      delay: (_target, index) => Math.min(index ?? 0, 4) * 18,
      ease: "out(4)",
    });

    return () => {
      animation.cancel();
      kids.forEach((kid) => {
        kid.style.removeProperty("opacity");
        kid.style.removeProperty("transform");
      });
    };
  }, [key]);
  return ref;
}
