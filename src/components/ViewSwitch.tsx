import { useCallback, useEffect, useRef, type KeyboardEvent } from "react";
import { animate } from "animejs";

import { NAV, PRIMARY_VIEWS, type ViewId } from "@/nav";
import { reducedMotion } from "@/lib/anime";
import { cn } from "@/lib/utils";

interface ViewSwitchProps {
  view: ViewId;
  navigate: (v: ViewId) => void;
}

/**
 * The switch carrying every place you actually work: Home, Macros, Watch,
 * Autopilot. One physical thumb slides between the segments: it stretches
 * across the gap on the way and settles into the target, so the eye tracks a
 * moving object instead of boxes swapping highlight.
 *
 * Settings is the one view not in here, and while it shows, no segment is
 * current: the thumb hides rather than lying about where you are.
 */
export function ViewSwitch({ view, navigate }: ViewSwitchProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const thumbRef = useRef<HTMLSpanElement>(null);
  const glowRef = useRef<HTMLSpanElement>(null);
  const btnRefs = useRef<(HTMLButtonElement | null)[]>([]);
  /** Where the thumb was last sent: the start point of the next trip. */
  const placed = useRef<{ x: number; w: number } | null>(null);

  const index = PRIMARY_VIEWS.findIndex((v) => v.id === view);

  const place = useCallback(
    (animated: boolean) => {
      const thumb = thumbRef.current;
      if (!thumb) return;

      const btn = index < 0 ? null : btnRefs.current[index];
      if (!btn) {
        thumb.style.opacity = "0";
        placed.current = null;
        return;
      }

      const x = btn.offsetLeft;
      const w = btn.offsetWidth;
      const from = placed.current;
      // A resize that changed nothing must not stomp on a trip in flight.
      if (!animated && from && from.x === x && from.w === w) return;
      placed.current = { x, w };
      thumb.style.opacity = "1";

      if (!animated || !from || reducedMotion()) {
        thumb.style.width = `${w}px`;
        thumb.style.transform = `translateX(${x}px)`;
        return;
      }

      const rightward = x > from.x;
      const stretch = rightward ? x + w - from.x : from.x + from.w - x;
      animate(thumb, {
        width: [
          { to: `${stretch}px`, duration: 170, ease: "out(2)" },
          { to: `${w}px`, duration: 340, ease: "out(3)" },
        ],
        translateX: rightward
          ? [
              { to: from.x, duration: 170 },
              { to: x, duration: 340, ease: "out(3)" },
            ]
          : [
              { to: x, duration: 170, ease: "out(2)" },
              { to: x, duration: 340 },
            ],
      });

      if (glowRef.current) {
        animate(glowRef.current, {
          opacity: [0.5, 0],
          scale: [0.85, 1.06],
          duration: 620,
          ease: "out(3)",
        });
      }
      const icon = btn.querySelector("svg");
      if (icon) {
        animate(icon, {
          scale: [1, 1.3, 1],
          rotate: [0, rightward ? 12 : -12, 0],
          duration: 520,
          ease: "out(3)",
        });
      }
    },
    [index],
  );

  useEffect(() => place(true), [place]);

  // Labels can reflow (font swap, a longer word after a locale change), so the
  // thumb re-measures instead of trusting the width it was born with.
  useEffect(() => {
    const track = trackRef.current;
    if (!track || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => place(false));
    ro.observe(track);
    btnRefs.current.forEach((b) => b && ro.observe(b));
    return () => ro.disconnect();
  }, [place]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const step = e.key === "ArrowRight" ? 1 : -1;
    const from = index < 0 ? (step > 0 ? -1 : PRIMARY_VIEWS.length) : index;
    const next = Math.min(PRIMARY_VIEWS.length - 1, Math.max(0, from + step));
    btnRefs.current[next]?.focus();
    navigate(PRIMARY_VIEWS[next].id);
  };

  return (
    <div
      ref={trackRef}
      role="group"
      aria-label="Switch view"
      onKeyDown={onKeyDown}
      className="relative isolate flex items-center gap-1 rounded-xl border border-border/60 bg-muted/40 p-1"
    >
      <span
        ref={thumbRef}
        aria-hidden="true"
        style={{ opacity: 0 }}
        className="pointer-events-none absolute top-1 bottom-1 left-0 -z-10 rounded-lg bg-background shadow-sm ring-1 ring-border/70"
      >
        <span
          ref={glowRef}
          className="absolute inset-0 rounded-lg bg-primary/30 opacity-0"
        />
      </span>

      {PRIMARY_VIEWS.map((v, i) => {
        const active = i === index;
        const accel = NAV.findIndex((n) => n.id === v.id) + 1;
        return (
          <button
            key={v.id}
            ref={(el) => {
              btnRefs.current[i] = el;
            }}
            type="button"
            aria-current={active ? "page" : undefined}
            aria-label={v.label}
            title={`${v.label}  ·  Alt+${accel}`}
            onClick={() => navigate(v.id)}
            className={cn(
              "relative flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-sm font-medium transition-colors duration-200 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 md:px-3.5",
              active ? "text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
          >
            <v.Icon className="size-4" />
            {/* Four labels don't fit a narrow window: below `md` the icons carry
                it alone, and the thumb re-measures through the ResizeObserver. */}
            <span className="hidden md:inline">{v.label}</span>
          </button>
        );
      })}
    </div>
  );
}
