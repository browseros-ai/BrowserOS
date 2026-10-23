"use client";
// beui.dev/components/motion/tooltip

import { AnimatePresence } from "motion/react";
import {
  type ComponentProps,
  type PointerEvent as ReactPointerEvent,
  cloneElement,
  createContext,
  isValidElement,
  type ReactElement,
  type ReactNode,
  type Ref,
  type RefObject,
  useCallback,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { TooltipSurface } from "@/components/motion/tooltip-surface";
import { useDismiss } from "@/lib/hooks/use-dismiss";
import { useHoverGesture } from "@/lib/hooks/use-hover-gesture";
import { useTapGesture } from "@/lib/hooks/use-tap-gesture";
import { cn } from "@/lib/utils";
import { focusFromKeyboard, graceArea, inBox, inPolygon, watchModality } from "./tooltip/grace";
import { TooltipArrow, TooltipPositioner } from "./tooltip/positioner";
import {
  type TooltipAlign,
  type TooltipAnchor,
  type TooltipPoint,
  type TooltipSide,
  useTooltipPointer,
} from "./tooltip/use-position";

export type { TooltipAlign, TooltipSide };

/* -------------------------------------------------------------------------- */
/* Provider                                                                   */
/* -------------------------------------------------------------------------- */

interface TooltipTiming {
  /** Delay before a hover opens a tooltip (ms). */
  delay: number;
  /** Delay before a tooltip closes once the pointer has left it (ms). */
  closeDelay: number;
  /** After one tooltip closes, others open instantly for this long (ms). */
  skipDelay: number;
}

const TimingContext = createContext<TooltipTiming>({ delay: 120, closeDelay: 0, skipDelay: 300 });

export interface TooltipProviderProps extends Partial<TooltipTiming> {
  children: ReactNode;
}

/** Shared timing for every tooltip inside it. Optional: tooltips work without one. */
export function TooltipProvider({ children, ...timing }: TooltipProviderProps) {
  const parent = useContext(TimingContext);
  const { delay = parent.delay, closeDelay = parent.closeDelay, skipDelay = parent.skipDelay } =
    timing;
  const value = useMemo(() => ({ delay, closeDelay, skipDelay }), [delay, closeDelay, skipDelay]);
  return <TimingContext.Provider value={value}>{children}</TimingContext.Provider>;
}

/* -------------------------------------------------------------------------- */
/* Root                                                                       */
/* -------------------------------------------------------------------------- */

// Moving along a toolbar should feel instant after the first label, and only
// one label may show at a time: a keyboard-focused control and a hovered one
// would otherwise both claim the screen.
let lastClosedAt = 0;
let current: { close: () => void } | null = null;

function isEmpty(content: ReactNode) {
  return content == null || content === false || content === "";
}

interface TooltipContextValue {
  open: boolean;
  contentId: string;
  labelId: string;
  asLabel: boolean;
  empty: boolean;
  followCursor: boolean;
  anchor: TooltipAnchor | null;
  /** True when a TooltipTrigger owns the anchor; false for an external `anchorRef`. */
  triggered: boolean;
  floatingRef: RefObject<HTMLSpanElement | null>;
  pointer: ReturnType<typeof useTooltipPointer>;
  setTrigger: (element: TooltipAnchor | null) => void;
  setEmpty: (empty: boolean) => void;
  close: () => void;
  onContentEnter: () => void;
  onContentLeave: (event: ReactPointerEvent) => void;
}

const TooltipContext = createContext<TooltipContextValue | null>(null);

function useTooltipContext(part: string) {
  const context = useContext(TooltipContext);
  if (!context) throw new Error(`${part} must be used inside <Tooltip>`);
  return context;
}

export interface TooltipRootProps {
  children?: ReactNode;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Delay before showing on hover (ms). Keyboard focus shows immediately. */
  delay?: number;
  /** Delay before hiding once the pointer has left (ms). */
  closeDelay?: number;
  /** Never show, e.g. while the label it carries is already visible. */
  disabled?: boolean;
  /**
   * Use the tooltip as the trigger's accessible name rather than its
   * description. For icon-only controls whose only label is the tooltip.
   */
  asLabel?: boolean;
  /** Follow real pointer coordinates; keyboard focus still uses the anchor. */
  followCursor?: boolean;
  /** Existing anchor for controlled integrations such as chart cells. */
  anchorRef?: RefObject<TooltipAnchor | null>;
  /** Id of the tooltip surface, for anchors that reference it themselves. */
  id?: string;
}

function TooltipRoot({
  children,
  open: controlledOpen,
  defaultOpen = false,
  onOpenChange,
  delay: delayProp,
  closeDelay: closeDelayProp,
  disabled = false,
  asLabel = false,
  followCursor = false,
  anchorRef,
  id,
  initialEmpty = false,
}: TooltipRootProps & { initialEmpty?: boolean }) {
  const timing = useContext(TimingContext);
  const delay = delayProp ?? timing.delay;
  const closeDelay = closeDelayProp ?? timing.closeDelay;
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const open = controlledOpen ?? uncontrolledOpen;
  const [empty, setEmpty] = useState(initialEmpty);
  const [trigger, setTrigger] = useState<TooltipAnchor | null>(null);
  const [external, setExternal] = useState<TooltipAnchor | null>(null);
  const anchor = anchorRef ? external : trigger;
  const generatedId = useId();
  const contentId = id ?? `${generatedId}-content`;
  const labelId = `${generatedId}-label`;
  const floatingRef = useRef<HTMLSpanElement | null>(null);
  const anchorObject = useRef<TooltipAnchor | null>(null);
  const pointer = useTooltipPointer(anchor, followCursor);

  // An external ref can attach after this component renders; read it once it has.
  useLayoutEffect(() => {
    if (!anchorRef) return;
    const element = anchorRef.current ?? null;
    if (element !== external) setExternal(element);
  });

  // Everything the event listeners read, current without re-binding them.
  const latest = useRef({ open, delay, closeDelay, disabled, empty, onOpenChange, controlled: false });
  useLayoutEffect(() => {
    anchorObject.current = anchor;
    latest.current = {
      open,
      delay,
      closeDelay,
      disabled,
      empty,
      onOpenChange,
      controlled: controlledOpen !== undefined,
    };
  });

  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const grace = useRef<(() => void) | null>(null);
  const self = useRef<{ close: () => void }>({ close: () => {} });

  const clearTimers = useCallback(() => {
    if (openTimer.current) clearTimeout(openTimer.current);
    if (closeTimer.current) clearTimeout(closeTimer.current);
    openTimer.current = null;
    closeTimer.current = null;
  }, []);

  const stopGrace = useCallback(() => {
    grace.current?.();
    grace.current = null;
  }, []);

  // Only a real change reaches the consumer: a pointer leaving a closed
  // tooltip is not news.
  const commit = useCallback((next: boolean) => {
    if (latest.current.open === next) return;
    if (!latest.current.controlled) setUncontrolledOpen(next);
    latest.current.onOpenChange?.(next);
  }, []);

  const openNow = useCallback(() => {
    clearTimers();
    stopGrace();
    if (latest.current.disabled || latest.current.empty) return;
    if (current && current !== self.current) current.close();
    current = self.current;
    commit(true);
  }, [clearTimers, stopGrace, commit]);

  const close = useCallback(() => {
    clearTimers();
    stopGrace();
    if (latest.current.open) lastClosedAt = Date.now();
    if (current === self.current) current = null;
    commit(false);
  }, [clearTimers, stopGrace, commit]);
  useLayoutEffect(() => {
    self.current.close = close;
  }, [close]);

  const requestOpen = useCallback(() => {
    clearTimers();
    if (latest.current.open || latest.current.disabled || latest.current.empty) return;
    const warm = Date.now() - lastClosedAt < timing.skipDelay;
    if (warm || latest.current.delay <= 0) openNow();
    else openTimer.current = setTimeout(openNow, latest.current.delay);
  }, [clearTimers, openNow, timing.skipDelay]);

  const requestClose = useCallback(() => {
    clearTimers();
    if (latest.current.closeDelay > 0) {
      closeTimer.current = setTimeout(close, latest.current.closeDelay);
    } else close();
  }, [clearTimers, close]);

  // Hold the label open while the pointer crosses from one box toward the other.
  const startGrace = useCallback(
    (exit: TooltipPoint, from: Element, to: Element | null) => {
      stopGrace();
      if (!to) return requestClose();
      const target = to.getBoundingClientRect();
      const origin = from.getBoundingClientRect();
      const corridor = graceArea(exit, target);
      const onMove = (event: PointerEvent) => {
        const point = { x: event.clientX, y: event.clientY };
        if (inBox(point, target) || inBox(point, origin)) return;
        if (!inPolygon(point, corridor)) requestClose();
      };
      document.addEventListener("pointermove", onMove);
      grace.current = () => document.removeEventListener("pointermove", onMove);
    },
    [stopGrace, requestClose],
  );

  const hover = useHoverGesture();
  const tap = useTapGesture<boolean>();

  // Listeners go on the trigger itself rather than on cloned props, so a
  // component that owns its own handlers keeps them, and no wrapper element
  // sits between the control and its layout.
  useEffect(() => {
    if (anchorRef || !trigger) return;
    watchModality();
    // A mouse press dismisses the label and keeps it away until the pointer
    // leaves; hovering a control you just clicked should not bring it back.
    let suppressed = false;
    // Focus that came from the keyboard holds the label open on its own.
    let keyboardFocus = false;

    const onEnter = (event: PointerEvent) => {
      if (!hover.enter(event)) return;
      stopGrace();
      if (closeTimer.current) clearTimeout(closeTimer.current);
      if (!suppressed) requestOpen();
    };
    const onLeave = (event: PointerEvent) => {
      if (!hover.leave(event)) return;
      suppressed = false;
      if (keyboardFocus) return;
      if (openTimer.current) {
        clearTimers();
        return;
      }
      if (latest.current.open && !followCursor) {
        startGrace({ x: event.clientX, y: event.clientY }, trigger, floatingRef.current);
      } else requestClose();
    };
    const onDown = (event: PointerEvent) => {
      tap.start(event, latest.current.open);
      if (event.pointerType === "mouse") {
        suppressed = true;
        close();
      }
    };
    // A finger never hovers, and Safari does not focus a button on tap either,
    // so the label is only reachable if the tap itself toggles it. The click
    // carries no pointerType; the pointerdown before it says whether it was a
    // tap, and keyboard activation has none at all.
    const onClick = () => {
      const gesture = tap.take();
      if (!gesture || gesture.pointerType === "mouse") return;
      if (gesture.state) close();
      else openNow();
    };
    const onKey = () => tap.drop();
    const onFocusIn = () => {
      if (!focusFromKeyboard()) return;
      keyboardFocus = true;
      openNow();
    };
    const onFocusOut = (event: FocusEvent) => {
      if (trigger.contains(event.relatedTarget as Node | null)) return;
      keyboardFocus = false;
      close();
    };

    // Both element kinds dispatch the same events; one type keeps the listeners typed.
    const el = trigger as HTMLElement;
    el.addEventListener("pointerenter", onEnter);
    el.addEventListener("pointerleave", onLeave);
    el.addEventListener("pointerdown", onDown);
    el.addEventListener("pointercancel", tap.drop);
    el.addEventListener("click", onClick);
    el.addEventListener("keydown", onKey);
    el.addEventListener("focusin", onFocusIn);
    el.addEventListener("focusout", onFocusOut);
    return () => {
      el.removeEventListener("pointerenter", onEnter);
      el.removeEventListener("pointerleave", onLeave);
      el.removeEventListener("pointerdown", onDown);
      el.removeEventListener("pointercancel", tap.drop);
      el.removeEventListener("click", onClick);
      el.removeEventListener("keydown", onKey);
      el.removeEventListener("focusin", onFocusIn);
      el.removeEventListener("focusout", onFocusOut);
    };
  }, [
    anchorRef,
    trigger,
    followCursor,
    hover,
    tap,
    requestOpen,
    requestClose,
    openNow,
    close,
    clearTimers,
    stopGrace,
    startGrace,
  ]);

  // While open, a trigger that turns disabled loses focus without a blur event,
  // and a window that loses focus never reports the pointer leaving.
  useEffect(() => {
    if (!open || anchorRef || !trigger) return;
    const observer = new MutationObserver(() => {
      if (trigger.matches(":disabled, [aria-disabled='true']")) close();
    });
    observer.observe(trigger, { attributes: true, attributeFilter: ["disabled", "aria-disabled"] });
    window.addEventListener("blur", close);
    return () => {
      observer.disconnect();
      window.removeEventListener("blur", close);
    };
  }, [open, anchorRef, trigger, close]);

  // A chart drives its own open state and swaps content as it goes; only a
  // trigger-owned label closes itself for having nothing to say.
  useEffect(() => {
    if (disabled || (empty && !anchorRef)) close();
  }, [disabled, empty, anchorRef, close]);

  useEffect(
    () => () => {
      clearTimers();
      stopGrace();
      if (current === self.current) current = null;
    },
    [clearTimers, stopGrace],
  );

  const insideTooltip = useCallback(
    (target: Element) => Boolean(floatingRef.current?.contains(target)),
    [],
  );
  // ...and closed by the next tap that lands somewhere else, or Escape. The
  // label covers nothing interactive, so that tap passes through to what it hit.
  useDismiss(open, close, anchorObject, { ignore: insideTooltip });

  const onContentEnter = useCallback(() => {
    stopGrace();
    clearTimers();
  }, [stopGrace, clearTimers]);

  const onContentLeave = useCallback(
    (event: ReactPointerEvent) => {
      const floating = floatingRef.current;
      if (!floating) return;
      if (anchorRef) {
        // Bridge the small gap back onto a chart; it reopens on its own terms.
        clearTimers();
        closeTimer.current = setTimeout(close, 100);
        return;
      }
      startGrace({ x: event.clientX, y: event.clientY }, floating, anchorObject.current);
    },
    [anchorRef, clearTimers, close, startGrace],
  );

  const value = useMemo<TooltipContextValue>(
    () => ({
      open,
      contentId,
      labelId,
      asLabel,
      empty,
      followCursor,
      anchor,
      triggered: !anchorRef,
      floatingRef,
      pointer,
      setTrigger,
      setEmpty,
      close,
      onContentEnter,
      onContentLeave,
    }),
    [
      open,
      contentId,
      labelId,
      asLabel,
      empty,
      followCursor,
      anchor,
      anchorRef,
      pointer,
      close,
      onContentEnter,
      onContentLeave,
    ],
  );

  return <TooltipContext.Provider value={value}>{children}</TooltipContext.Provider>;
}

/* -------------------------------------------------------------------------- */
/* Trigger                                                                    */
/* -------------------------------------------------------------------------- */

function mergeIds(...ids: unknown[]) {
  return ids.filter((value) => typeof value === "string" && value).join(" ") || undefined;
}

function assignRef<T>(ref: Ref<T> | undefined, value: T | null) {
  if (typeof ref === "function") ref(value);
  else if (ref) (ref as { current: T | null }).current = value;
}

export interface TooltipTriggerProps extends ComponentProps<"button"> {
  /** Use the single child element as the trigger instead of rendering a button. */
  asChild?: boolean;
}

export function TooltipTrigger({ asChild = false, children, ref, ...props }: TooltipTriggerProps) {
  const { open, contentId, labelId, asLabel, empty, setTrigger } =
    useTooltipContext("TooltipTrigger");
  const child = asChild && isValidElement(children) ? children : null;
  const childProps = (child?.props ?? props) as Record<string, unknown>;
  const childRef = child ? (childProps.ref as Ref<TooltipAnchor> | undefined) : undefined;
  const attached = useRef<TooltipAnchor | null>(null);

  const setRef = useCallback(
    (element: TooltipAnchor | null) => {
      attached.current = element;
      setTrigger(element);
      assignRef(childRef ?? (ref as Ref<TooltipAnchor> | undefined), element);
    },
    [setTrigger, childRef, ref],
  );

  useEffect(() => {
    if (process.env.NODE_ENV !== "production" && child && !attached.current) {
      console.warn(
        "TooltipTrigger: the child did not attach a ref, so the tooltip has nothing to listen to. Render a DOM element or a component that passes `ref` through.",
      );
    }
  }, [child]);

  const aria = asLabel
    ? {
        "aria-labelledby": empty
          ? (childProps["aria-labelledby"] as string | undefined)
          : labelId,
      }
    : {
        "aria-describedby": mergeIds(
          childProps["aria-describedby"],
          open && !empty ? contentId : undefined,
        ),
      };

  if (asChild) {
    if (!child) return children;
    return cloneElement(child as ReactElement<Record<string, unknown>>, { ref: setRef, ...aria });
  }

  return (
    <button
      type="button"
      data-state={open ? "open" : "closed"}
      {...props}
      ref={setRef as Ref<HTMLButtonElement>}
      {...aria}
    >
      {children}
    </button>
  );
}

/* -------------------------------------------------------------------------- */
/* Content                                                                    */
/* -------------------------------------------------------------------------- */

export interface TooltipContentProps {
  children?: ReactNode;
  side?: TooltipSide;
  align?: TooltipAlign;
  /** Gap between anchor and tooltip (px). */
  sideOffset?: number;
  /** Draw an arrow pointing at the anchor. */
  arrow?: boolean;
  /** Point within the anchor, as fractions of its rendered width and height. */
  anchorPoint?: TooltipPoint;
  className?: string;
}

export function TooltipContent({
  children,
  side = "top",
  align = "center",
  sideOffset,
  arrow = false,
  anchorPoint,
  className,
}: TooltipContentProps) {
  const {
    open,
    contentId,
    labelId,
    asLabel,
    followCursor,
    anchor,
    floatingRef,
    pointer,
    setEmpty,
    close,
    onContentEnter,
    onContentLeave,
  } = useTooltipContext("TooltipContent");
  const empty = isEmpty(children);
  const arrowRef = useRef<SVGSVGElement | null>(null);

  useLayoutEffect(() => {
    setEmpty(empty);
  }, [empty, setEmpty]);

  if (typeof document === "undefined") return null;

  return createPortal(
    <>
      {/* The name has to exist while the label is closed, so it lives apart from the surface. */}
      {asLabel && !empty ? (
        <span id={labelId} hidden>
          {children}
        </span>
      ) : null}
      <AnimatePresence>
        {open && !empty ? (
          <TooltipPositioner
            key="tooltip"
            anchor={anchor}
            floatingRef={floatingRef}
            arrowRef={arrow ? arrowRef : undefined}
            anchorPoint={anchorPoint}
            followCursor={followCursor}
            side={side}
            align={align}
            sideOffset={sideOffset}
            onDismiss={close}
            pointer={pointer}
          >
            {(positioned, isPresent, placedSide) => (
              <TooltipSurface
                id={asLabel ? undefined : contentId}
                aria-hidden={asLabel || undefined}
                ready={positioned}
                onPointerEnter={onContentEnter}
                onPointerLeave={onContentLeave}
                style={{ pointerEvents: isPresent && !followCursor ? "auto" : "none" }}
                className={cn(
                  "max-w-[min(20rem,calc(100vw-16px))] whitespace-normal break-words",
                  arrow && "relative",
                  className,
                )}
              >
                {children}
                {arrow ? <TooltipArrow ref={arrowRef} side={placedSide} /> : null}
              </TooltipSurface>
            )}
          </TooltipPositioner>
        ) : null}
      </AnimatePresence>
    </>,
    document.body,
  );
}

/* -------------------------------------------------------------------------- */
/* Tooltip (root, or one-line shorthand when given `content`)                 */
/* -------------------------------------------------------------------------- */

export interface TooltipProps
  extends TooltipRootProps,
    Omit<TooltipContentProps, "children" | "className"> {
  /**
   * Shorthand: the tooltip's content. With it, `children` is the trigger
   * element; without it, compose `TooltipTrigger` and `TooltipContent`.
   */
  content?: ReactNode;
  /** Shorthand only: classes for the tooltip surface. */
  className?: string;
}

export function Tooltip(props: TooltipProps) {
  if (!("content" in props)) return <TooltipRoot {...props} />;
  const {
    content,
    children,
    side,
    align,
    sideOffset,
    arrow,
    anchorPoint,
    className,
    ...root
  } = props;
  if (!root.anchorRef && !isValidElement(children)) return children ?? null;
  return (
    <TooltipRoot {...root} initialEmpty={isEmpty(content)}>
      {isValidElement(children) ? <TooltipTrigger>{children}</TooltipTrigger> : null}
      <TooltipContent
        side={side}
        align={align}
        sideOffset={sideOffset}
        arrow={arrow}
        anchorPoint={anchorPoint}
        className={className}
      >
        {content}
      </TooltipContent>
    </TooltipRoot>
  );
}
