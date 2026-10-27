"use client";
// beui.dev/components/motion/popover

import {
  AnimatePresence,
  animate,
  motion,
  useMotionValue,
  usePresence,
  useReducedMotion,
  useTransform,
} from "motion/react";
import {
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
import { usePopoverPortalPosition } from "@/components/motion/popover-position";
import { EASE_OUT } from "@/lib/ease";
import { FOCUSABLE_SELECTOR } from "@/lib/focusable";
import { cn } from "@/lib/utils";

type Side = "top" | "bottom";
type Align = "start" | "end";

type MorphContextValue = {
  open: boolean;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  triggerId: string;
  contentId: string;
  /** The element the panel measures against — see `registerTrigger`. */
  triggerRef: React.MutableRefObject<HTMLElement | null>;
  registerTrigger: (node: HTMLElement | null) => void;
  contentRef: React.MutableRefObject<HTMLDivElement | null>;
  /** Arms or disarms moving focus into the panel as it next appears. */
  armFocusOnOpen: (armed: boolean) => void;
  /** Reads and disarms the flag set by `armFocusOnOpen`. */
  takeFocusOnOpen: () => boolean;
};

const MorphContext = createContext<MorphContextValue | null>(null);

function useMorphContext(component: string) {
  const ctx = useContext(MorphContext);
  if (!ctx) throw new Error(`${component} must be used within <MorphPopover>`);
  return ctx;
}

export interface MorphPopoverProps {
  children: ReactNode;
  /** Controlled open state. */
  open?: boolean;
  /** Uncontrolled initial open state. */
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  className?: string;
}

/**
 * A popover whose full-size panel reveals from the corner nearest the trigger
 * on one spring, then retraces the same clip on close. Content and surface
 * edges keep their size throughout. Closes on outside pointer / Escape.
 * Controlled or uncontrolled.
 */
export function MorphPopover({
  children,
  open: controlledOpen,
  defaultOpen = false,
  onOpenChange,
  className,
}: MorphPopoverProps) {
  const baseId = useId();
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  const [trigger, setTrigger] = useState<HTMLElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const controlled = controlledOpen !== undefined;
  const open = controlled ? controlledOpen : internalOpen;

  const setOpen = useCallback(
    (next: boolean) => {
      if (!controlled) setInternalOpen(next);
      onOpenChange?.(next);
    },
    [controlled, onOpenChange],
  );
  const toggle = useCallback(() => setOpen(!open), [setOpen, open]);

  // A keyboard open arms focus for the panel to take as it appears. Any later
  // key or pointer press disarms it, so an open the parent declined cannot
  // leave it armed for a later open that nobody asked to focus.
  const focusOnOpenRef = useRef(false);
  const disarmRef = useRef<AbortController | null>(null);
  const armFocusOnOpen = useCallback((armed: boolean) => {
    disarmRef.current?.abort();
    disarmRef.current = null;
    focusOnOpenRef.current = armed;
    if (!armed) return;
    const controller = new AbortController();
    const disarm = () => {
      focusOnOpenRef.current = false;
      controller.abort();
    };
    const options = { capture: true, signal: controller.signal };
    document.addEventListener("keydown", disarm, options);
    document.addEventListener("pointerdown", disarm, options);
    disarmRef.current = controller;
  }, []);
  const takeFocusOnOpen = useCallback(() => {
    const armed = focusOnOpenRef.current;
    armFocusOnOpen(false);
    return armed;
  }, [armFocusOnOpen]);
  useEffect(() => () => disarmRef.current?.abort(), []);

  // A trigger normally registers itself through MorphPopoverTrigger. It can't
  // when something else already clones the element — a Tooltip wrapping the
  // button, say — and an unregistered trigger leaves the panel with nothing to
  // measure against, so it renders permanently invisible. The root boxes the
  // trigger exactly (the content portals out of it), so it stands in until a
  // real trigger registers, and stands in again if that one unmounts. Both are
  // state, so a trigger arriving while the panel is open re-anchors it.
  const anchorRef = useMemo<React.MutableRefObject<HTMLElement | null>>(
    () => ({ current: trigger ?? root }),
    [root, trigger],
  );

  // The panel is a `role="dialog"` and goes inert the moment it closes, so
  // focus cannot be left sitting inside it: a dismissal hands it back to the
  // trigger, the way the ARIA dialog pattern asks. A pointer dismissal takes
  // the focus onward itself when it lands on something focusable — this only
  // catches the case where it would otherwise be stranded. When no trigger has
  // registered, the root anchor stands in only if it can actually hold focus;
  // there is nowhere better than where the keyboard already is, so leave it.
  const close = useCallback(() => {
    setOpen(false);
    const focused = document.activeElement;
    const inPanel =
      focused instanceof HTMLElement && contentRef.current?.contains(focused);
    if (!inPanel) return;
    const restore = trigger ?? (root && root.tabIndex >= 0 ? root : null);
    restore?.focus();
  }, [root, setOpen, trigger]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    const onPointer = (e: PointerEvent) => {
      const target = e.target as Node;
      if (
        root &&
        !root.contains(target) &&
        !contentRef.current?.contains(target)
      )
        close();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onPointer);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onPointer);
    };
  }, [open, root, close]);

  const ctx = useMemo<MorphContextValue>(
    () => ({
      open,
      setOpen,
      toggle,
      triggerId: `${baseId}-trigger`,
      contentId: `${baseId}-content`,
      triggerRef: anchorRef,
      registerTrigger: setTrigger,
      contentRef,
      armFocusOnOpen,
      takeFocusOnOpen,
    }),
    [open, setOpen, toggle, baseId, anchorRef, armFocusOnOpen, takeFocusOnOpen],
  );

  return (
    <MorphContext.Provider value={ctx}>
      <div ref={setRoot} className={cn("relative inline-flex", className)}>
        {children}
      </div>
    </MorphContext.Provider>
  );
}

export interface MorphPopoverTriggerProps {
  children: ReactElement;
}

function mergeRefs<T>(...refs: Array<Ref<T> | undefined>) {
  return (node: T | null) => {
    for (const ref of refs) {
      if (typeof ref === "function") ref(node);
      else if (ref && typeof ref === "object")
        (ref as React.MutableRefObject<T | null>).current = node;
    }
  };
}

/** Wraps a single element, toggling the popover on click. */
export function MorphPopoverTrigger({ children }: MorphPopoverTriggerProps) {
  const ctx = useMorphContext("MorphPopoverTrigger");
  const child = children as ReactElement<Record<string, unknown>>;
  const childOnClick = child?.props?.onClick as
    | ((e: unknown) => void)
    | undefined;
  const childOnKeyDown = child?.props?.onKeyDown as
    | ((e: unknown) => void)
    | undefined;
  const childOnPointerDown = child?.props?.onPointerDown as
    | ((e: unknown) => void)
    | undefined;
  // Set by an Enter or Space keydown on the trigger, cleared by a pointer
  // press, read by the click that follows.
  const keyboardRef = useRef(false);
  const childRef = (child?.props as { ref?: Ref<HTMLElement> } | undefined)
    ?.ref;
  // Register once per actual ref change, not once per open-state render.
  const mergedRef = useMemo(
    () => mergeRefs(childRef, ctx.registerTrigger),
    [childRef, ctx.registerTrigger],
  );
  if (!isValidElement(children)) return children;

  return cloneElement(child, {
    id: ctx.triggerId,
    ref: mergedRef,
    onKeyDown: (e: unknown) => {
      childOnKeyDown?.(e);
      const key = (e as { key?: string } | undefined)?.key;
      keyboardRef.current = key === "Enter" || key === " ";
    },
    onPointerDown: (e: unknown) => {
      childOnPointerDown?.(e);
      keyboardRef.current = false;
    },
    onClick: (e: unknown) => {
      childOnClick?.(e);
      // Enter and Space reach here as a click with `detail` 0 after their
      // keydown on this element. A pointer click counts at least 1, and a
      // scripted click() has no keydown before it. Only a keyboard open
      // moves focus into the panel.
      const keyboard =
        keyboardRef.current &&
        (e as { detail?: number } | undefined)?.detail === 0;
      keyboardRef.current = false;
      ctx.armFocusOnOpen(!ctx.open && keyboard);
      ctx.toggle();
    },
    "aria-haspopup": "dialog",
    "aria-expanded": ctx.open,
    "aria-controls": ctx.open ? ctx.contentId : undefined,
  });
}

// Full-distance clip travel needs a shorter curve than SPRING_PANEL's slow
// overdamped tail. A non-bouncing spring reaches its rest within 260 ms.
const MORPH_ENTER = { type: "spring", duration: 0.26, bounce: 0 } as const;
const MORPH_EXIT = { duration: 0.2, ease: EASE_OUT } as const;

// Keep the panel and content at their final size. An inset clip reveals the
// nearest corner without scaling borders, shadows or keyboard-focus geometry.
// A full-size clip keeps content at its final coordinates without box-to-box
// scale correction. One progress value keeps clip and opacity synchronized;
// clip-path and the shadow may paint, so this is not a compositor-only morph.
function clipAt(side: Side, align: Align, radius: number, inset: number) {
  return `inset(${side === "top" ? inset : 0}% ${align === "start" ? inset : 0}% ${side === "bottom" ? inset : 0}% ${align === "end" ? inset : 0}% round ${radius}px)`;
}

export interface MorphPopoverContentProps {
  children: ReactNode;
  side?: Side;
  align?: Align;
  /** Gap between trigger and panel, in px. Default 8. */
  sideOffset?: number;
  /** Panel corner radius, in px. Default 8. */
  radius?: number;
  /** Draw the surface shadow. Default true. */
  shadow?: boolean;
  /**
   * Runs after the portalled surface is positioned and visible. Passing it
   * hands focus to the caller: the panel then moves no focus on its own.
   */
  onOpenAutoFocus?: (content: HTMLDivElement) => void;
  /**
   * Element to focus when the trigger opens the panel by keyboard. Defaults
   * to the first focusable element in the panel, or the panel itself. A
   * pointer open does not move focus. Ignored with `onOpenAutoFocus`.
   */
  initialFocus?: RefObject<HTMLElement | null>;
  className?: string;
}

export function MorphPopoverContent(props: MorphPopoverContentProps) {
  const ctx = useMorphContext("MorphPopoverContent");
  const [portalReady, setPortalReady] = useState(false);
  useEffect(() => setPortalReady(true), []);
  if (!portalReady) return null;
  return createPortal(
    <AnimatePresence>
      {ctx.open && <MorphPopoverSurface {...props} />}
    </AnimatePresence>,
    document.body,
  );
}

// Measurement belongs to the mounted portal session: reopening must not start
// an entrance at the previous session's coordinates before measuring this one.
function MorphPopoverSurface({
  children,
  side = "bottom",
  align = "end",
  sideOffset = 8,
  radius = 8,
  shadow = true,
  onOpenAutoFocus,
  initialFocus,
  className,
}: MorphPopoverContentProps) {
  const ctx = useMorphContext("MorphPopoverContent");
  const reduce = useReducedMotion() ?? false;
  const [isPresent, safeToRemove] = usePresence();
  const layout = usePopoverPortalPosition(
    ctx.triggerRef,
    ctx.contentRef,
    isPresent,
  );

  const preferredLeft = layout
    ? align === "end"
      ? layout.trigger.left + layout.trigger.width - layout.content.width
      : layout.trigger.left
    : 0;
  // Keep larger compositions, such as calendars, inside the viewport. Prefer
  // the requested side, then use the roomier side when it cannot fit there.
  const gutter = 12;
  const below = layout
    ? Math.max(
        0,
        window.innerHeight -
          layout.trigger.top -
          layout.trigger.height -
          sideOffset -
          gutter,
      )
    : 0;
  const above = layout
    ? Math.max(0, layout.trigger.top - sideOffset - gutter)
    : 0;
  const requestedSpace = side === "bottom" ? below : above;
  const otherSpace = side === "bottom" ? above : below;
  const resolvedSide =
    layout &&
    layout.content.height > requestedSpace &&
    otherSpace > requestedSpace
      ? side === "bottom"
        ? "top"
        : "bottom"
      : side;
  const availableHeight = resolvedSide === "bottom" ? below : above;
  const left = layout
    ? Math.max(
        gutter,
        Math.min(
          preferredLeft,
          window.innerWidth - layout.content.width - gutter,
        ),
      )
    : 0;
  const top = layout
    ? resolvedSide === "bottom"
      ? layout.trigger.top + layout.trigger.height + sideOffset
      : Math.max(
          gutter,
          layout.trigger.top - layout.content.height - sideOffset,
        )
    : 0;

  const progress = useMotionValue(0);
  const clipPath = useTransform(progress, (p) =>
    clipAt(resolvedSide, align, radius, 92 * (1 - p)),
  );
  const surfaceOpacity = useTransform(progress, [0, 0.15], [0, 1]);
  // Reduced motion has no travel, so a short fade carries the whole entrance
  // and exit on its own.
  const fade = useMotionValue(reduce ? 0 : 1);
  const ready = layout !== null;
  const focusedOnOpen = useRef(false);
  useEffect(() => {
    if (
      !ready ||
      !isPresent ||
      focusedOnOpen.current ||
      !ctx.contentRef.current
    )
      return;
    focusedOnOpen.current = true;
    onOpenAutoFocus?.(ctx.contentRef.current);
  }, [ready, isPresent, ctx.contentRef, onOpenAutoFocus]);
  // `safeToRemove` is a new function on every render: AnimatePresence
  // rebuilds its context each time its parent renders. Read it through a ref
  // so a render during the exit (a consumer's state change, a list streaming
  // in) does not restart the exit from where it got to — restarted every
  // render, it would close in ever smaller steps and never report completion,
  // leaving an inert, invisible panel mounted for good.
  const safeToRemoveRef = useRef(safeToRemove);
  useLayoutEffect(() => {
    safeToRemoveRef.current = safeToRemove;
  });
  useEffect(() => {
    if (!ready) {
      if (!isPresent) safeToRemoveRef.current?.();
      return;
    }
    const settle = () => {
      if (!isPresent) safeToRemoveRef.current?.();
    };
    if (reduce) {
      progress.jump(1);
      const animation = animate(fade, isPresent ? 1 : 0, {
        duration: 0.12,
        onComplete: settle,
      });
      return () => animation.stop();
    }
    const animation = animate(progress, isPresent ? 1 : 0, {
      ...(isPresent ? MORPH_ENTER : MORPH_EXIT),
      onComplete: settle,
    });
    return () => animation.stop();
  }, [fade, progress, ready, isPresent, reduce]);

  // The wrapper stays `visibility: hidden` until the first measurement, and
  // focus() inside a hidden subtree is silently ignored. So move focus in the
  // commit that makes the panel visible, not when it mounts. A CSS transition
  // on `visibility` can still hold the controls hidden for a moment after
  // that commit (a reduced-motion reset that gives every element a short
  // transition-duration does), so while focus rests on the panel or has not
  // moved, each such transition that ends tries the candidates again, until
  // the user presses a key or pointer. The effect runs once per open, so it
  // reads the caller's props through refs: a new ref or callback passed
  // mid-open must not tear down a pending retry.
  const initialFocusRef = useRef(initialFocus);
  const callerFocusesRef = useRef(onOpenAutoFocus !== undefined);
  useLayoutEffect(() => {
    initialFocusRef.current = initialFocus;
    callerFocusesRef.current = onOpenAutoFocus !== undefined;
  });
  const { contentRef, takeFocusOnOpen } = ctx;
  useLayoutEffect(() => {
    if (!ready || !isPresent) return;
    if (!takeFocusOnOpen() || callerFocusesRef.current) return;
    const panel = contentRef.current;
    const wrapper = panel?.parentElement;
    if (!panel || !wrapper) return;
    const from = document.activeElement;
    const unmoved = () =>
      document.activeElement === from || document.activeElement === panel;
    // The selector also matches elements focus() refuses: a hidden input, a
    // disabled button that carries a tabindex, anything under display: none.
    // Keep the first candidate that takes focus, else hold it on the panel.
    const focusFirst = () => {
      const candidates = [
        initialFocusRef.current?.current,
        ...panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      ];
      for (const candidate of candidates) {
        if (!candidate) continue;
        candidate.focus({ preventScroll: true });
        if (unmoved()) continue;
        // preventScroll keeps the page still, but it also stops the panel's
        // own scroll area from bringing a row below the fold into view.
        candidate.scrollIntoView?.({ block: "nearest" });
        return true;
      }
      panel.focus({ preventScroll: true });
      return false;
    };
    if (focusFirst()) return;
    const retry = (event: TransitionEvent) => {
      if (event.propertyName !== "visibility") return;
      if (!unmoved() || focusFirst()) stop();
    };
    const stop = () => {
      wrapper.removeEventListener("transitionend", retry);
      document.removeEventListener("keydown", stop, true);
      document.removeEventListener("pointerdown", stop, true);
    };
    wrapper.addEventListener("transitionend", retry);
    document.addEventListener("keydown", stop, true);
    document.addEventListener("pointerdown", stop, true);
    return stop;
  }, [ready, isPresent, contentRef, takeFocusOnOpen]);

  return (
    <motion.div
      data-morph-popover-portal=""
      inert={!isPresent}
      style={{
        left,
        top,
        opacity: reduce ? fade : surfaceOpacity,
        pointerEvents: isPresent ? "auto" : "none",
        visibility: layout ? "visible" : "hidden",
      }}
      // A drop-shadow follows the clipped surface; box-shadow would be cut
      // off by its own clip. Caller surface classes retain their old target.
      className={cn(
        "fixed z-[9999]",
        shadow && "[filter:drop-shadow(0_10px_18px_rgba(0,0,0,0.14))]",
      )}
    >
      <motion.div
        ref={ctx.contentRef}
        id={ctx.contentId}
        role="dialog"
        aria-labelledby={ctx.triggerId}
        tabIndex={-1}
        style={{
          clipPath: reduce ? undefined : clipPath,
          borderRadius: radius,
          maxHeight: layout ? availableHeight : undefined,
          overflowX: "hidden",
          overflowY: "auto",
        }}
        className={cn(
          // The panel holds focus when nothing inside it can. An inset ring
          // stays inside the clip that would cut off an outline.
          "overflow-hidden border border-border bg-background outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
          className,
        )}
      >
        {children}
      </motion.div>
    </motion.div>
  );
}
