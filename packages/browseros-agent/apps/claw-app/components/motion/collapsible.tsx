"use client";
// beui.dev/components/motion/collapsible

import { ChevronDown } from "lucide-react";
import { type HTMLMotionProps, isValidMotionProp, motion, useReducedMotion } from "motion/react";
import {
  cloneElement,
  createContext,
  type ReactElement,
  type Ref,
  type RefObject,
  useCallback,
  useContext,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { EASE_OUT, SPRING_LAYOUT } from "@/lib/ease";
import { cn } from "@/lib/utils";

export type CollapsibleState = {
  open: boolean;
  disabled: boolean;
  setOpen: (open: boolean) => void;
  toggle: () => void;
};

type CollapsibleContextValue = CollapsibleState & {
  reduce: boolean;
  contentId: string;
  triggerId: string;
  triggerRef: RefObject<HTMLButtonElement | null>;
};

const CollapsibleContext = createContext<CollapsibleContextValue | null>(null);

function useCollapsibleContext(component: string) {
  const context = useContext(CollapsibleContext);
  if (!context) throw new Error(`${component} must be used within <Collapsible>`);
  return context;
}

/** Shared state and actions for custom children inside a Collapsible root. */
export function useCollapsible(): CollapsibleState {
  return useCollapsibleContext("useCollapsible");
}

function mergeRefs<T>(...refs: Array<Ref<T> | undefined>) {
  return (node: T | null) => {
    const cleanups = refs.map((ref) => {
      if (typeof ref === "function") return ref(node);
      if (ref) ref.current = node;
      return undefined;
    });
    if (!node) return;
    return () => {
      refs.forEach((ref, index) => {
        const cleanup = cleanups[index];
        if (typeof cleanup === "function") cleanup();
        else if (typeof ref === "function") ref(null);
        else if (ref) ref.current = null;
      });
    };
  };
}

export interface CollapsibleProps extends Omit<HTMLMotionProps<"div">, "layout"> {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  disabled?: boolean;
  /** Layout classes for the inner wrapper, separate from the animated surface. */
  contentClassName?: string;
}

export function Collapsible({
  open: controlledOpen,
  defaultOpen = false,
  onOpenChange,
  disabled = false,
  className,
  contentClassName,
  children,
  style,
  ...props
}: CollapsibleProps) {
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const open = controlledOpen ?? internalOpen;
  const reduce = useReducedMotion() ?? false;
  const id = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const stateRef = useRef({ open, controlled: controlledOpen !== undefined, disabled, onOpenChange });
  useLayoutEffect(() => {
    stateRef.current = { open, controlled: controlledOpen !== undefined, disabled, onOpenChange };
  }, [controlledOpen, disabled, onOpenChange, open]);
  const setOpen = useCallback((next: boolean) => {
    const state = stateRef.current;
    if (state.disabled || next === state.open) return;
    if (!state.controlled) {
      stateRef.current = { ...state, open: next };
      setInternalOpen(next);
    }
    state.onOpenChange?.(next);
  }, []);
  const toggle = useCallback(() => setOpen(!stateRef.current.open), [setOpen]);
  const context = useMemo<CollapsibleContextValue>(
    () => ({ open, disabled, reduce, contentId: `${id}-content`, triggerId: `${id}-trigger`, triggerRef, setOpen, toggle }),
    [disabled, id, open, reduce, setOpen, toggle],
  );

  return (
    <CollapsibleContext.Provider value={context}>
      <motion.div
        {...props}
        layout={!reduce}
        transition={{ layout: SPRING_LAYOUT }}
        data-slot="collapsible"
        data-state={open ? "open" : "closed"}
        data-disabled={disabled || undefined}
        className={cn("relative w-full", className)}
        style={{ ...style, originY: 0 }}
      >
        <motion.div
          layout={reduce ? false : "position"}
          transition={{ layout: SPRING_LAYOUT }}
          style={{ originY: 0 }}
          className={cn("min-w-0", contentClassName)}
        >
          {children}
        </motion.div>
      </motion.div>
    </CollapsibleContext.Provider>
  );
}

export interface CollapsibleTriggerProps extends Omit<HTMLMotionProps<"button">, "id" | "layout"> {
  /** Reuse a button element or a component that forwards its props and ref to a button. */
  render?: ReactElement;
}

export function CollapsibleTrigger({
  className,
  children,
  onClick,
  disabled,
  ref,
  render,
  style,
  ...props
}: CollapsibleTriggerProps) {
  const context = useCollapsibleContext("CollapsibleTrigger");
  const child = render as ReactElement<HTMLMotionProps<"button">> | undefined;
  const childRef = child?.props.ref;
  const mergedRef = useMemo(
    () => mergeRefs(context.triggerRef, ref, childRef),
    [context.triggerRef, ref, childRef],
  );
  const unavailable = context.disabled || disabled || child?.props.disabled;
  const handleClick: NonNullable<HTMLMotionProps<"button">["onClick"]> = (event) => {
    child?.props.onClick?.(event);
    onClick?.(event);
    if (!event.defaultPrevented && !unavailable) context.toggle();
  };
  const triggerProps = {
    id: context.triggerId,
    type: "button" as const,
    disabled: unavailable,
    "aria-expanded": context.open,
    "aria-controls": context.contentId,
    "data-slot": "collapsible-trigger",
    "data-state": context.open ? "open" : "closed",
    "data-disabled": unavailable || undefined,
    onClick: handleClick,
    ref: mergedRef,
  };

  if (child) {
    if (typeof child.type === "string" && child.type !== "button") {
      throw new Error("CollapsibleTrigger render must be a button or a component that renders a button.");
    }
    const forwarded = typeof child.type === "string"
      ? Object.fromEntries(Object.entries(props).filter(([key]) => !isValidMotionProp(key)))
      : props;
    return cloneElement(child, {
      ...forwarded,
      ...triggerProps,
      className: cn(className, child.props.className),
      style: { ...style, ...child.props.style },
      children: children === undefined ? child.props.children : children,
    });
  }

  return (
    <motion.button
      {...props}
      {...triggerProps}
      style={style}
      layout={context.reduce ? false : "position"}
      transition={{ layout: SPRING_LAYOUT }}
      className={cn(
        "flex min-h-10 w-full items-center gap-3 rounded-xl px-3 py-2 text-left text-sm font-medium outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
        className,
      )}
    >
      {children}
    </motion.button>
  );
}

export interface CollapsibleContentProps extends Omit<HTMLMotionProps<"div">, "id" | "layout" | "animate" | "initial"> {
  /** Padding and layout classes for the measured content, inside the clipping wrapper. */
  contentClassName?: string;
}

export function CollapsibleContent({
  className,
  contentClassName,
  children,
  ref,
  style,
  ...props
}: CollapsibleContentProps) {
  const context = useCollapsibleContext("CollapsibleContent");
  const contentRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(0);
  const mergedRef = useMemo(() => mergeRefs(contentRef, ref), [ref]);

  useLayoutEffect(() => {
    const node = innerRef.current;
    if (!node) return;
    const measure = () => setHeight(node.offsetHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (!context.open && contentRef.current?.contains(document.activeElement)) {
      context.triggerRef.current?.focus({ preventScroll: true });
    }
  }, [context.open, context.triggerRef]);

  return (
    <motion.div
      {...props}
      ref={mergedRef}
      id={context.contentId}
      role="region"
      aria-labelledby={context.triggerId}
      aria-hidden={!context.open}
      inert={!context.open}
      data-slot="collapsible-content"
      data-state={context.open ? "open" : "closed"}
      initial={false}
      layout={!context.reduce}
      transition={{ layout: SPRING_LAYOUT }}
      // A one-pixel closed box keeps transform projection continuous at zero.
      // Geometry changes once; the inner layout projection keeps text sharp.
      style={{ ...style, height: context.open ? height || "auto" : 1, originY: 0 }}
      className={cn("overflow-hidden", className)}
    >
      <motion.div
        ref={innerRef}
        layout={!context.reduce}
        style={{ originY: 0 }}
        initial={false}
        animate={{ opacity: context.open ? 1 : 0 }}
        transition={{ opacity: { duration: context.open ? 0.18 : 0.12, ease: EASE_OUT }, layout: SPRING_LAYOUT }}
        className={cn("flow-root", contentClassName)}
      >
        {children}
      </motion.div>
    </motion.div>
  );
}

export interface CollapsibleIndicatorProps extends HTMLMotionProps<"span"> {}

export function CollapsibleIndicator({ className, children, ...props }: CollapsibleIndicatorProps) {
  const { open, reduce } = useCollapsibleContext("CollapsibleIndicator");
  return (
    <motion.span
      {...props}
      aria-hidden="true"
      data-slot="collapsible-indicator"
      initial={false}
      animate={{ transform: `rotate(${open ? 180 : 0}deg)` }}
      transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
      className={cn("ml-auto inline-flex shrink-0 items-center justify-center text-muted-foreground", className)}
    >
      {children ?? <ChevronDown className="size-4" />}
    </motion.span>
  );
}
