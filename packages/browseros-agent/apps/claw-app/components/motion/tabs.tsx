"use client";
// beui.dev/components/motion/tabs

import { ChevronLeft, ChevronRight } from "lucide-react";
import { cancelFrame, frame, motion, MotionConfig, useReducedMotion, useSpring, type MotionValue, type Transition } from "motion/react";
import {
  createContext,
  useCallback,
  useContext,
  useId,
  useLayoutEffect,
  useRef,
  useMemo,
  useState,
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { EASE_OUT, SPRING_SWAP } from "@/lib/ease";
import { cn } from "@/lib/utils";

type Variant = "pill" | "underline" | "segment";
/** Whether trigger icons show on every tab or only on the selected one. */
type IconDisplay = "always" | "selected";

type Ctx = {
  value: string;
  setValue: (v: string) => void;
  layoutId: string;
  variant: Variant;
  icons: IconDisplay;
  /** The pill's box in the list, for tabs whose width changes as they're selected. */
  track: Track;
  /** Whether the tracked pill has been measured; until then CSS places it. */
  placed: boolean;
  setPlaced: (placed: boolean) => void;
  /**
   * The one tab Tab reaches: the selected tab, or the first enabled one when
   * nothing enabled is selected. `null` until the triggers have registered,
   * when every enabled tab stays reachable.
   */
  tabStop: string | null;
  registerTab: (value: string, disabled: boolean) => void;
  unregisterTab: (value: string) => void;
  /** Values with a mounted panel, so triggers only point at panels that exist. */
  panels: ReadonlySet<string>;
  registerPanel: (value: string) => void;
  unregisterPanel: (value: string) => void;
  tabId: (value: string) => string;
  panelId: (value: string) => string;
};

type Track = Record<"x" | "y" | "width" | "height", MotionValue<number>>;

const TabsCtx = createContext<Ctx | null>(null);

function useTabs() {
  const ctx = useContext(TabsCtx);
  if (!ctx) throw new Error("Tabs.* must be used inside <Tabs>");
  return ctx;
}

// Settle without overshoot: a scrollable tab list would turn even a small
// overshoot into a transient scrollbar and layout shift.
// Scale stiffness and damping together for a quicker glide with the same feel.
const transition: Transition = {
  type: "spring",
  stiffness: 245,
  damping: 36,
  mass: 1.2,
};

export function Tabs({
  defaultValue,
  value,
  onValueChange,
  variant = "pill",
  icons = "always",
  children,
  className,
}: {
  defaultValue?: string;
  value?: string;
  onValueChange?: (v: string) => void;
  variant?: Variant;
  /**
   * With "selected", a trigger's icon springs open only on the selected tab
   * and folds away when another is chosen. Defaults to "always".
   */
  icons?: IconDisplay;
  children: ReactNode;
  className?: string;
}) {
  const [internal, setInternal] = useState(defaultValue ?? "");
  const layoutId = useId();
  const reduce = useReducedMotion();
  const controlled = value !== undefined;
  const current = controlled ? value : internal;
  const setValue = useCallback(
    (v: string) => {
      if (!controlled) setInternal(v);
      onValueChange?.(v);
    },
    [controlled, onValueChange],
  );
  const x = useSpring(0, transition);
  const y = useSpring(0, transition);
  const width = useSpring(0, transition);
  const height = useSpring(0, transition);
  const track = useMemo(() => ({ x, y, width, height }), [x, y, width, height]);
  const [placed, setPlaced] = useState(false);
  const [tabs, setTabs] = useState<Map<string, boolean>>(new Map());
  const registerTab = useCallback((tab: string, disabled: boolean) => {
    setTabs((map) => (map.get(tab) === disabled ? map : new Map(map).set(tab, disabled)));
  }, []);
  const unregisterTab = useCallback((tab: string) => {
    setTabs((map) => {
      if (!map.has(tab)) return map;
      const next = new Map(map);
      next.delete(tab);
      return next;
    });
  }, []);
  const [panels, setPanels] = useState<ReadonlySet<string>>(new Set());
  const registerPanel = useCallback((panel: string) => {
    setPanels((set) => (set.has(panel) ? set : new Set(set).add(panel)));
  }, []);
  const unregisterPanel = useCallback((panel: string) => {
    setPanels((set) => {
      if (!set.has(panel)) return set;
      const next = new Set(set);
      next.delete(panel);
      return next;
    });
  }, []);
  // Resolved during render so the list has exactly one tab stop in the same
  // commit that changes the selection or the tabs.
  let tabStop: string | null = null;
  if (tabs.size > 0) {
    const enabled = Array.from(tabs).filter(([, disabled]) => !disabled);
    tabStop = enabled.some(([tab]) => tab === current) ? current : (enabled[0]?.[0] ?? "");
  }
  const baseId = useId();
  const tabId = useCallback((tab: string) => `${baseId}-tab-${tab.replace(/[^\w-]/g, "_")}`, [baseId]);
  const panelId = useCallback((tab: string) => `${baseId}-panel-${tab.replace(/[^\w-]/g, "_")}`, [baseId]);
  const contextValue = useMemo(
    () => ({
      value: current,
      setValue,
      layoutId,
      variant,
      icons,
      track,
      placed,
      setPlaced,
      tabStop,
      registerTab,
      unregisterTab,
      panels,
      registerPanel,
      unregisterPanel,
      tabId,
      panelId,
    }),
    [current, icons, layoutId, setValue, variant, track, placed, tabStop, registerTab, unregisterTab, panels, registerPanel, unregisterPanel, tabId, panelId],
  );
  return (
    <MotionConfig transition={reduce ? { duration: 0 } : transition}>
      <TabsCtx.Provider value={contextValue}>
        {/* layoutRoot: the indicator's layoutId measures in page coordinates, so
            inside fixed/scrolled containers it would replay scroll offsets as
            movement. The pill only ever travels within the list, so scoping
            projection to the Tabs wrapper is always correct. */}
        <motion.div layoutRoot className={className}>
          {children}
        </motion.div>
      </TabsCtx.Provider>
    </MotionConfig>
  );
}

const listClasses: Record<Variant, string> = {
  pill: "inline-flex items-center gap-1 rounded-full bg-card p-1",
  underline: "inline-flex items-center gap-1 border-b border-border",
  segment: "inline-flex items-center gap-0 rounded-lg bg-card p-0.5",
};

export function TabsList({
  children,
  className,
  wrapperClassName,
  onKeyDown,
  ...props
}: {
  wrapperClassName?: string;
} & HTMLAttributes<HTMLDivElement>) {
  const { variant, value, icons, track, placed, setPlaced } = useTabs();
  const reduce = useReducedMotion();
  const tracked = icons === "selected" && variant !== "underline";
  const rootRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const viewportId = useId();
  const [edges, setEdges] = useState({ overflow: false, left: false, right: false });

  const measure = useCallback(() => {
    const root = rootRef.current;
    const viewport = viewportRef.current;
    if (!root || !viewport) return;
    // Overlay controls do not reduce the viewport or change its scroll range.
    const overflow = viewport.scrollWidth > root.clientWidth + 1;
    const max = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
    const rtl = getComputedStyle(viewport).direction === "rtl";
    // Modern browsers expose negative scrollLeft in RTL. Clamp rubber-banding.
    const fromLeft = Math.max(0, Math.min(max, rtl ? max + viewport.scrollLeft : viewport.scrollLeft));
    const next = { overflow, left: fromLeft > 1, right: fromLeft < max - 1 };
    setEdges((previous) => previous.overflow === next.overflow && previous.left === next.left && previous.right === next.right ? previous : next);
  }, []);

  const reveal = useCallback((tab: HTMLElement | null) => {
    const viewport = viewportRef.current;
    if (!viewport || !tab) return;
    const frame = viewport.getBoundingClientRect();
    const item = tab.getBoundingClientRect();
    const max = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
    const rtl = getComputedStyle(viewport).direction === "rtl";
    const fromLeft = Math.max(0, Math.min(max, rtl ? max + viewport.scrollLeft : viewport.scrollLeft));
    // Keep the selected/focused label clear of the arrows over the faded edges.
    const left = frame.left + (fromLeft > 1 ? 36 : 0);
    const right = frame.right - (fromLeft < max - 1 ? 36 : 0);
    const delta = item.left < left ? item.left - left : item.right > right ? item.right - right : 0;
    // Scroll only this viewport; scrollIntoView can also move the whole page.
    if (delta) viewport.scrollBy({ left: delta, behavior: reduce ? "instant" : "smooth" });
  }, [reduce]);

  useLayoutEffect(() => {
    const root = rootRef.current;
    const viewport = viewportRef.current;
    const list = listRef.current;
    if (!root || !viewport || !list) return;
    const update = () => {
      measure();
      reveal(list.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]'));
    };
    const observer = new ResizeObserver(update);
    observer.observe(root);
    observer.observe(viewport);
    observer.observe(list);
    viewport.addEventListener("scroll", measure, { passive: true });
    update();
    return () => {
      observer.disconnect();
      viewport.removeEventListener("scroll", measure);
    };
  }, [measure, reveal]);

  useLayoutEffect(() => {
    // Children may change without a resize; controlled selection must also reveal.
    void children;
    void value;
    void edges.overflow;
    measure();
    reveal(listRef.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]') ?? null);
  }, [children, value, edges.overflow, measure, reveal]);

  useLayoutEffect(() => {
    // With icons="selected" the chosen tab grows as its icon opens and the
    // last one shrinks. A layout animation measures the tabs mid-change, so
    // the pill would squeeze and catch up. Instead the pill springs straight
    // to where the selected tab will settle, with every icon open or closed.
    if (!tracked) return;
    const list = listRef.current;
    if (!list) return;
    void children;
    const place = () => {
      const tabs = Array.from(list.querySelectorAll<HTMLElement>('[role="tab"]'));
      const target = tabs.find((tab) => tab.dataset.tabsValue === value)?.parentElement;
      if (!target) return;
      // Measure the settled layout: open the selected icon and fold the rest
      // for one synchronous read, then hand the styles back to Motion before
      // anything paints. Exact in any layout, from flex rows to grids.
      const icons = Array.from(list.querySelectorAll<HTMLElement>('[role="tab"] > [data-tabs-icon]'));
      const saved = icons.map((icon) => [icon.style.width, icon.style.marginRight]);
      for (const icon of icons) {
        const open = icon.parentElement?.parentElement === target;
        icon.style.width = open ? "auto" : "0px";
        icon.style.marginRight = open ? `${ICON_GAP}px` : "0px";
      }
      const box = {
        x: target.offsetLeft,
        y: target.offsetTop,
        // Fractional: a rounded width leaves the pill a hair off its tab.
        width: target.getBoundingClientRect().width,
        height: target.offsetHeight,
      };
      icons.forEach((icon, index) => {
        [icon.style.width, icon.style.marginRight] = saved[index];
      });
      // A first measurement, or one after the list was hidden, lands in place.
      const instant = reduce || track.width.get() === 0;
      for (const key of ["x", "y", "width", "height"] as const) {
        if (instant) track[key].jump(box[key]);
        else track[key].set(box[key]);
      }
      // Hand over from the CSS pill once there's a real box, before paint.
      if (box.width > 0) setPlaced(true);
    };
    place();
    const observer = new ResizeObserver(place);
    for (const tab of list.querySelectorAll('[role="tab"]')) if (tab.parentElement) observer.observe(tab.parentElement);
    return () => observer.disconnect();
  }, [tracked, value, children, reduce, track, setPlaced]);

  useLayoutEffect(() => {
    if (variant === "underline") return;
    const list = listRef.current;
    if (!list) return;
    void children;
    // `placed` swaps the indicator element, so measure the new one.
    void placed;
    const labels = Array.from(list.querySelectorAll<HTMLElement>("[data-tabs-label]"));
    const indicator = list.querySelector<HTMLElement>("[data-tabs-indicator]");
    const target = list.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
    if (!indicator || !target || target.dataset.tabsValue !== value) {
      for (const label of labels) label.style.clipPath = "inset(0 100% 0 0)";
      return;
    }
    let frames = 0;
    let stillFrames = 0;
    let previous: { left: number; right: number } | undefined;
    const syncClips = () => {
      const pill = (reduce ? target : indicator).getBoundingClientRect();
      // Read ALL geometry before writing ANY masks. A loop per tab interleaved
      // reads and writes, forcing the browser to flush styles repeatedly.
      const clips = labels.map((label) => {
        const bounds = label.getBoundingClientRect();
        const left = Math.max(0, Math.min(bounds.width, pill.left - bounds.left));
        const right = Math.max(0, Math.min(bounds.width, bounds.right - pill.right));
        return left + right >= bounds.width ? "inset(0 100% 0 0)" : `inset(0 ${right}px 0 ${left}px)`;
      });
      labels.forEach((label, index) => {
        if (label.style.clipPath !== clips[index]) label.style.clipPath = clips[index];
      });
      frames += 1;
      stillFrames = previous && Math.abs(pill.left - previous.left) < 0.01 && Math.abs(pill.right - previous.right) < 0.01 ? stillFrames + 1 : 0;
      previous = { left: pill.left, right: pill.right };
      if (reduce || (frames > 2 && stillFrames >= 2)) cancelFrame(syncClips);
    };
    // One shared pass after Motion paints the projected pill keeps every label
    // in sync, including labels crossed during a long or interrupted glide.
    frame.postRender(syncClips, true);
    // A list measured while hidden (display: none, a closed panel, a
    // breakpoint) gets every mask closed. Sync again once it has a size, or
    // the selected label stays invisible until the value changes.
    const resync = () => {
      frames = 0;
      stillFrames = 0;
      previous = undefined;
      cancelFrame(syncClips);
      frame.postRender(syncClips, true);
    };
    const observer = new ResizeObserver(resync);
    observer.observe(list);
    return () => {
      observer.disconnect();
      cancelFrame(syncClips);
    };
  }, [value, children, variant, reduce, placed]);

  // Arrow keys move along the list and select as focus arrives, wrapping at
  // the ends; Home and End jump to the first and last enabled tab.
  const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(event);
    if (event.defaultPrevented) return;
    const list = listRef.current;
    if (!list || !(event.target instanceof HTMLElement)) return;
    const tabs = Array.from(list.querySelectorAll<HTMLButtonElement>('[role="tab"]')).filter(
      (tab) => !tab.disabled && tab.closest('[role="tablist"]') === list,
    );
    const at = tabs.indexOf(event.target as HTMLButtonElement);
    if (at < 0) return;
    const rtl = getComputedStyle(list).direction === "rtl";
    const forward = rtl ? "ArrowLeft" : "ArrowRight";
    const back = rtl ? "ArrowRight" : "ArrowLeft";
    const next =
      event.key === forward
        ? tabs[(at + 1) % tabs.length]
        : event.key === back
          ? tabs[(at - 1 + tabs.length) % tabs.length]
          : event.key === "Home"
            ? tabs[0]
            : event.key === "End"
              ? tabs[tabs.length - 1]
              : null;
    if (!next) return;
    event.preventDefault();
    next.focus();
    next.click();
  };

  const scroll = (direction: number) => {
    const viewport = viewportRef.current;
    if (viewport) viewport.scrollBy({ left: direction * viewport.clientWidth * 0.8, behavior: reduce ? "instant" : "smooth" });
  };
  const controlClass = "absolute inset-y-0 z-20 inline-flex w-9 items-center justify-center text-foreground transition-opacity hover:opacity-70 focus-visible:outline-2 focus-visible:-outline-offset-4 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-0";
  const surfaceClass = variant === "pill" ? "rounded-full bg-card" : variant === "segment" ? "rounded-lg bg-card" : "";

  return (
    <div ref={rootRef} className={cn("relative isolate flex w-full max-w-full min-w-0 items-center", edges.overflow && surfaceClass, wrapperClassName)}>
      {edges.overflow && (
        <button type="button" aria-label="Scroll tabs left" aria-controls={viewportId} disabled={!edges.left} onClick={() => scroll(-1)} className={cn(controlClass, "left-0 rounded-l-full")}>
          <ChevronLeft size={20} aria-hidden="true" />
        </button>
      )}
      <motion.div
        ref={viewportRef}
        id={viewportId}
        layoutScroll
        className={cn("w-full min-w-0 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden", edges.overflow && "[border-radius:inherit]")}
        style={edges.overflow ? {
          maskImage: `linear-gradient(to right, ${edges.left ? "transparent, black 40px" : "black, black 0px"}, ${edges.right ? "black calc(100% - 40px), transparent" : "black 100%"})`,
        } : undefined}
        onFocusCapture={(event) => {
          if (event.target instanceof HTMLElement && event.target.getAttribute("role") === "tab") reveal(event.target);
        }}
      >
        <div {...props} ref={listRef} role="tablist" onKeyDown={onListKeyDown} className={cn(listClasses[variant], "w-max", tracked && "relative", className)}>
          {children}
        </div>
      </motion.div>
      {edges.overflow && edges.left && (
        <span aria-hidden="true" className="pointer-events-none absolute inset-y-0 left-0 z-10 w-10 rounded-l-[inherit] backdrop-blur-[2px] [mask-image:linear-gradient(to_right,black,transparent)]" />
      )}
      {edges.overflow && edges.right && (
        <span aria-hidden="true" className="pointer-events-none absolute inset-y-0 right-0 z-10 w-10 rounded-r-[inherit] backdrop-blur-[2px] [mask-image:linear-gradient(to_left,black,transparent)]" />
      )}
      {edges.overflow && (
        <button type="button" aria-label="Scroll tabs right" aria-controls={viewportId} disabled={!edges.right} onClick={() => scroll(1)} className={cn(controlClass, "right-0 rounded-r-full")}>
          <ChevronRight size={20} aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

/** Space between a selected-only icon and its label, in pixels. */
const ICON_GAP = 6;

/**
 * A trigger's icon before its label. Shown on every tab, or with
 * `icons="selected"` springing open on the selected tab only.
 */
function TriggerIcon({ icon, active }: { icon: ReactNode; active: boolean }) {
  const { icons } = useTabs();
  const reduce = useReducedMotion();
  if (icons === "always") {
    return (
      <span aria-hidden="true" className="mr-1.5 inline-flex shrink-0 items-center">
        {icon}
      </span>
    );
  }
  return (
    <motion.span
      aria-hidden="true"
      data-tabs-icon=""
      initial={false}
      animate={{
        width: active ? "auto" : 0,
        marginRight: active ? ICON_GAP : 0,
        opacity: active ? 1 : 0,
        scale: active ? 1 : 0.6,
      }}
      transition={reduce ? { duration: 0 } : SPRING_SWAP}
      className="inline-flex shrink-0 items-center justify-center overflow-hidden"
    >
      {icon}
    </motion.span>
  );
}

export function TabsTrigger({
  value,
  icon,
  children,
  className,
  indicatorClassName,
  onClick,
  ...props
}: {
  value: string;
  /** An icon before the label, such as a Lucide icon element. */
  icon?: ReactNode;
  indicatorClassName?: string;
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, "value">) {
  const {
    value: current,
    setValue,
    layoutId,
    variant,
    icons,
    track,
    placed,
    tabStop,
    registerTab,
    unregisterTab,
    panels,
    tabId,
    panelId,
  } = useTabs();
  const active = current === value;
  const disabled = Boolean(props.disabled);
  useLayoutEffect(() => {
    registerTab(value, disabled);
  }, [registerTab, value, disabled]);
  useLayoutEffect(() => () => unregisterTab(value), [unregisterTab, value]);
  const a11y = {
    id: tabId(value),
    "aria-controls": panels.has(value) ? panelId(value) : undefined,
    tabIndex: !disabled && (tabStop === null || tabStop === value) ? 0 : -1,
  };
  const content = icon ? (
    <>
      <TriggerIcon icon={icon} active={active} />
      {children}
    </>
  ) : (
    children
  );
  // React owns the initial mask only; TabsList synchronizes subsequent masks.
  const [initialClip] = useState(() => active ? "inset(0)" : "inset(0 100% 0 0)");

  if (variant === "underline") {
    return (
      <button
        // Consumer ids and wiring win, e.g. several tabs sharing one panel.
        {...a11y}
        {...props}
        type="button"
        role="tab"
        aria-selected={active}
        onClick={(event) => {
          onClick?.(event);
          if (!event.defaultPrevented) setValue(value);
        }}
        className={cn(
          "relative isolate px-3 pb-2.5 pt-1 -mb-px text-sm font-medium transition-colors min-h-[44px] inline-flex items-center whitespace-nowrap shrink-0",
          active ? "text-foreground" : "text-muted-foreground hover:text-foreground",
          className,
        )}
      >
        {content}
        {active ? (
        <motion.span
          layoutId={layoutId}
          layout
          className={cn(
            "absolute bottom-0 left-0 right-0 h-px bg-primary",
            indicatorClassName,
          )}
        />
        ) : null}
      </button>
    );
  }

  const radius = variant === "pill" ? "rounded-full" : "rounded-md";
  // Tracked pills are placed in the list, so this wrapper must not be their
  // containing block. Server HTML can't measure, so until the first
  // measurement the pill fills the selected tab with CSS, like the others.
  const tracked = icons === "selected" && placed;

  return (
    <div className={cn("shrink-0", !tracked && "relative")}>
      {/* Distinct keys: reused across the swap, the CSS pill's leftover
          `transform: none` would hide the tracked pill's offset until the
          next value change. */}
      {active && tracked ? (
        <motion.span
          key="tracked"
          data-tabs-indicator=""
          style={{ ...track, borderRadius: variant === "pill" ? 9999 : 8 }}
          className={cn("absolute top-0 left-0 bg-primary", radius, indicatorClassName)}
        />
      ) : active ? (
        <motion.span
          key="layout"
          data-tabs-indicator=""
          layoutId={layoutId}
          layout
          style={{ borderRadius: variant === "pill" ? 9999 : 8 }}
          className={cn(
            "absolute inset-0 bg-primary",
            radius,
            indicatorClassName,
          )}
        />
      ) : null}
      <button
        // Consumer ids and wiring win, e.g. several tabs sharing one panel.
        {...a11y}
        {...props}
        type="button"
        role="tab"
        aria-selected={active}
        data-tabs-value={value}
        onClick={(event) => {
          onClick?.(event);
          if (!event.defaultPrevented) setValue(value);
        }}
        className={cn(
          "relative z-10 inline-flex items-center justify-center whitespace-nowrap bg-transparent px-3.5 py-1.5 text-sm font-medium outline-none",
          "text-muted-foreground hover:text-foreground",
          radius,
          className,
        )}
      >
        {content}
        <span
          data-tabs-label=""
          aria-hidden="true"
          inert
          className="pointer-events-none absolute inset-0 inline-flex items-center justify-center text-primary-foreground [gap:inherit] [padding:inherit]"
          style={{ clipPath: initialClip }}
        >
          {content}
        </span>
      </button>
    </div>
  );
}

export function TabsContent({ value, children, className }: { value: string; children: ReactNode; className?: string }) {
  const { value: current, registerPanel, unregisterPanel, tabId, panelId } = useTabs();
  const reduce = useReducedMotion();
  const active = current === value;
  useLayoutEffect(() => {
    registerPanel(value);
    return () => unregisterPanel(value);
  }, [registerPanel, unregisterPanel, value]);
  const a11y = {
    id: panelId(value),
    role: "tabpanel",
    "aria-labelledby": tabId(value),
  } as const;
  // Inactive panels stay mounted but hidden, so their content (e.g. source
  // code) is present in the server-rendered HTML for crawlers and assistive
  // tech, instead of being dropped from the DOM.
  if (!active) {
    return (
      <div {...a11y} hidden className={className}>
        {children}
      </div>
    );
  }
  return (
    <motion.div
      {...a11y}
      // Reachable with Tab so a panel without controls can still be read.
      tabIndex={0}
      key={value}
      initial={{ opacity: 0, y: reduce ? 0 : 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: EASE_OUT }}
      className={cn("mt-4 outline-none focus-visible:ring-2 focus-visible:ring-ring/40 rounded-md", className)}
    >
      {children}
    </motion.div>
  );
}
