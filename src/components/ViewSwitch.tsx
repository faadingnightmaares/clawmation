import {
  useCallback,
  useLayoutEffect,
  useRef,
  type KeyboardEvent,
} from "react";
import { animate, spring } from "animejs";

import {
  NAV,
  PRIMARY_VIEWS,
  VIEW_ICON_STROKE_WIDTH,
  type ViewId,
} from "@/nav";
import { reducedMotion } from "@/lib/anime";
import { cn } from "@/lib/utils";

interface ViewSwitchProps {
  view: ViewId;
  navigate: (v: ViewId) => void;
}

/** The primary work areas. Settings remains a separate utility action. */
export function ViewSwitch({ view, navigate }: ViewSwitchProps) {
  const switchRef = useRef<HTMLDivElement>(null);
  const indicatorRef = useRef<HTMLDivElement>(null);
  const refractionRef = useRef<HTMLDivElement>(null);
  const indicatorAnimationRef = useRef<{ cancel: () => void } | null>(null);
  const refractionAnimationRef = useRef<{ cancel: () => void } | null>(null);
  const indicatorGeometryRef = useRef<{ left: number; width: number } | null>(
    null,
  );
  const lastContainerWidthRef = useRef<number | null>(null);
  const btnRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const previousView = useRef<ViewId | null>(null);
  const index = PRIMARY_VIEWS.findIndex((v) => v.id === view);

  const positionIndicator = useCallback(
    (withMotion: boolean) => {
      const indicator = indicatorRef.current;
      const refraction = refractionRef.current;
      const target = btnRefs.current[index];
      if (!indicator) return;

      if (!target) {
        indicatorAnimationRef.current?.cancel();
        refractionAnimationRef.current?.cancel();
        indicatorAnimationRef.current = null;
        refractionAnimationRef.current = null;
        indicatorGeometryRef.current = null;
        indicator.style.opacity = "0";
        if (refraction) {
          refraction.style.opacity = "0";
          refraction.style.transform = "translateX(-130%)";
        }
        previousView.current = view;
        return;
      }

      const left = target.offsetLeft;
      const width = target.offsetWidth;
      const shouldAnimate =
        withMotion &&
        previousView.current !== null &&
        indicatorGeometryRef.current !== null &&
        !reducedMotion();
      const previousGeometry = indicatorGeometryRef.current;

      indicatorAnimationRef.current?.cancel();
      refractionAnimationRef.current?.cancel();
      indicatorAnimationRef.current = null;
      refractionAnimationRef.current = null;
      if (refraction) {
        refraction.style.opacity = "0";
        refraction.style.transform = "translateX(-130%)";
      }

      indicator.style.left = `${left}px`;
      indicator.style.width = `${width}px`;

      if (!shouldAnimate || !previousGeometry) {
        indicator.style.opacity = "1";
        indicator.style.transform =
          "translateX(0px) scaleX(1) scaleY(1)";
      } else {
        const translateFrom = previousGeometry.left - left;
        const scaleFrom = previousGeometry.width / Math.max(width, 1);
        indicatorAnimationRef.current = animate(indicator, {
          translateX: [translateFrom, 0],
          scaleX: [scaleFrom, 1],
          scaleY: [0.96, 1],
          opacity: [0.92, 1],
          duration: 360,
          ease: spring({ duration: 360, bounce: 0.06 }),
        });
        if (refraction) {
          refractionAnimationRef.current = animate(refraction, {
            translateX: ["-130%", "235%"],
            scaleX: [0.86, 1.04, 0.94],
            opacity: [0, 0.42, 0],
            duration: 300,
            delay: 15,
            ease: "inOut(3)",
          });
        }
      }

      indicatorGeometryRef.current = { left, width };
      previousView.current = view;
    },
    [index, view],
  );

  useLayoutEffect(() => {
    positionIndicator(previousView.current !== null);

    const readContainerWidth = () =>
      switchRef.current?.getBoundingClientRect().width ?? 0;
    lastContainerWidthRef.current = readContainerWidth();

    const reposition = () => {
      lastContainerWidthRef.current = readContainerWidth();
      positionIndicator(false);
    };
    window.addEventListener("resize", reposition);

    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            // Compare border-box to border-box. ResizeObserver's contentRect
            // excludes the switch border and otherwise cancels this transition
            // immediately on its first observation.
            const width = readContainerWidth();
            const previousWidth = lastContainerWidthRef.current;
            if (
              previousWidth !== null &&
              Math.abs(width - previousWidth) < 0.5
            ) {
              return;
            }
            lastContainerWidthRef.current = width;
            positionIndicator(false);
          });
    if (switchRef.current) resizeObserver?.observe(switchRef.current);

    return () => {
      indicatorAnimationRef.current?.cancel();
      refractionAnimationRef.current?.cancel();
      indicatorAnimationRef.current = null;
      refractionAnimationRef.current = null;
      window.removeEventListener("resize", reposition);
      resizeObserver?.disconnect();
    };
  }, [positionIndicator]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const step = e.key === "ArrowRight" ? 1 : -1;
    const from = index < 0 ? (step > 0 ? -1 : PRIMARY_VIEWS.length) : index;
    let next = from + step;
    while (
      next >= 0 &&
      next < PRIMARY_VIEWS.length &&
      PRIMARY_VIEWS[next].disabled
    ) {
      next += step;
    }
    if (next < 0 || next >= PRIMARY_VIEWS.length) return;
    btnRefs.current[next]?.focus();
    navigate(PRIMARY_VIEWS[next].id);
  };

  return (
    <div
      ref={switchRef}
      role="group"
      aria-label="Switch view"
      onKeyDown={onKeyDown}
      className="relative isolate flex items-center gap-1 rounded-[18px] border border-border/70 bg-card/75 p-1.5 shadow-[0_6px_24px_rgba(46,33,20,0.05)]"
    >
      <div
        ref={indicatorRef}
        aria-hidden="true"
        className="view-switch-indicator pointer-events-none absolute bottom-1.5 top-1.5 z-0 origin-left rounded-xl opacity-0"
      >
        <div
          ref={refractionRef}
          data-view-switch-refraction=""
          className="view-switch-refraction"
        />
      </div>
      {PRIMARY_VIEWS.map((item, itemIndex) => {
        const active = itemIndex === index;
        const accel = NAV.findIndex((entry) => entry.id === item.id) + 1;
        const title = item.disabled
          ? `${item.label} · ${item.badge ?? "Unavailable"}`
          : `${item.label} · Alt+${accel}`;

        return (
          <button
            key={item.id}
            ref={(element) => {
              btnRefs.current[itemIndex] = element;
            }}
            type="button"
            aria-current={active ? "page" : undefined}
            aria-disabled={item.disabled || undefined}
            aria-label={item.label}
            title={title}
            disabled={item.disabled}
            onClick={() => navigate(item.id)}
            className={cn(
              "relative z-10 flex h-9 items-center gap-1.5 rounded-xl px-2.5 text-sm font-medium transition-[color,background-color,transform] duration-150 ease-[cubic-bezier(0.16,1,0.3,1)] outline-none active:translate-y-px focus-visible:ring-[3px] focus-visible:ring-ring/35 disabled:translate-y-0 md:px-3.5",
              item.disabled
                ? "cursor-not-allowed text-muted-foreground/55"
                : active
                  ? "text-primary"
                  : "text-muted-foreground hover:bg-muted/55 hover:text-foreground",
            )}
          >
            <span className="grid size-[18px] shrink-0 place-items-center" aria-hidden="true">
              <item.Icon
                className="size-[17px]"
                strokeWidth={VIEW_ICON_STROKE_WIDTH}
              />
            </span>
            <span className="hidden lg:inline">{item.label}</span>
            {item.badge && (
              <span className="rounded border border-border/70 bg-background/55 px-1 py-0.5 text-[9px] font-semibold uppercase leading-none tracking-wide text-muted-foreground">
                {item.badge}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
