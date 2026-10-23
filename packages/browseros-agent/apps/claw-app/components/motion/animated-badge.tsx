"use client";
// beui.dev/components/motion/animated-badge

import {
  AlertTriangle,
  Check,
  Circle,
  Info,
  LoaderCircle,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  AnimatePresence,
  motion,
  useReducedMotion,
  type HTMLMotionProps,
  type Variants,
} from "motion/react";
import type { ReactNode } from "react";
import { EASE_OUT, SPRING_LAYOUT, SPRING_SWAP } from "@/lib/ease";
import { cn } from "@/lib/utils";

export type AnimatedBadgeStatus =
  | "neutral"
  | "info"
  | "success"
  | "warning"
  | "danger"
  | "loading";

export type AnimatedBadgeSize = "sm" | "md";

export interface AnimatedBadgeProps extends Omit<
  HTMLMotionProps<"span">,
  "children"
> {
  status?: AnimatedBadgeStatus;
  size?: AnimatedBadgeSize;
  children?: ReactNode;
  icon?: ReactNode;
  showIcon?: boolean;
  pulse?: boolean;
  contentKey?: string | number;
}

const STATUS_CLASS: Record<AnimatedBadgeStatus, string> = {
  neutral: "border-border bg-card text-muted-foreground",
  info: "border-primary/30 bg-muted text-primary",
  success: "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
  warning: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400",
  danger: "border-destructive/30 bg-destructive/10 text-destructive",
  loading: "border-primary/30 bg-muted text-primary",
};

const SIZE_CLASS: Record<AnimatedBadgeSize, string> = {
  sm: "h-6 gap-1.5 px-2 text-[11px]",
  md: "h-8 gap-2 px-3 text-xs",
};

const ICON_CLASS: Record<AnimatedBadgeSize, string> = {
  sm: "h-3 w-3",
  md: "h-3.5 w-3.5",
};

const ICONS: Record<AnimatedBadgeStatus, LucideIcon> = {
  neutral: Circle,
  info: Info,
  success: Check,
  warning: AlertTriangle,
  danger: X,
  loading: LoaderCircle,
};

// Status swaps roll like the action swap primitives: the leaving glyphs
// slide up and out while the next ones slide in from below. Both fade from
// fully transparent through a light blur, so the label never reads as a
// solid smudge sitting on top of the badge, and the exit outruns the entry.
const ROLL_EXIT_TRANSITION = { duration: 0.14, ease: EASE_OUT } as const;
const ROLL_BLUR = "blur(3px)";

const ICON_ROLL_VARIANTS: Variants = {
  initial: { opacity: 0, y: 12, filter: ROLL_BLUR },
  animate: {
    opacity: 1,
    y: 0,
    filter: "blur(0px)",
    transition: SPRING_SWAP,
  },
  exit: {
    opacity: 0,
    y: -12,
    filter: ROLL_BLUR,
    transition: ROLL_EXIT_TRANSITION,
  },
};

const TEXT_ROLL_VARIANTS: Variants = {
  initial: { opacity: 0, y: "90%", filter: ROLL_BLUR },
  animate: {
    opacity: 1,
    y: "0%",
    filter: "blur(0px)",
    transition: SPRING_SWAP,
  },
  exit: {
    opacity: 0,
    y: "-90%",
    filter: ROLL_BLUR,
    transition: ROLL_EXIT_TRANSITION,
  },
};

// Reduced motion swaps in place: the next content lands at rest and the
// leaving content hides on its first frame instead of overlapping it.
const REST = { opacity: 1, y: 0, filter: "blur(0px)" } as const;
const REST_EXIT = { opacity: 0, transition: { duration: 0 } } as const;

export function AnimatedBadge({
  status = "neutral",
  size = "md",
  children,
  icon,
  showIcon = true,
  pulse = status === "loading",
  contentKey,
  className,
  style,
  ...rest
}: AnimatedBadgeProps) {
  const reduce = useReducedMotion();
  // Reduced motion skips the layout projection altogether: a zero-duration
  // layout animation still paints one transformed frame.
  const slotLayout = reduce ? false : ("position" as const);
  const Icon = ICONS[status];
  // `0` is a real label; booleans and "" from conditional callers are not and
  // must not reserve the gap and an empty slot.
  const hasLabel = children != null && typeof children !== "boolean" && children !== "";
  const resolvedContentKey =
    contentKey ??
    (typeof children === "string" || typeof children === "number"
      ? children
      : status);

  return (
    <motion.span
      layout={!reduce}
      transition={SPRING_LAYOUT}
      style={style}
      className={cn(
        "relative inline-flex shrink-0 items-center overflow-hidden whitespace-nowrap rounded-full border font-medium tabular-nums",
        "transition-colors duration-300",
        STATUS_CLASS[status],
        SIZE_CLASS[size],
        className,
      )}
      {...rest}
    >
      {/* The pulse breathes through opacity alone. Scaling this layer would
          shrink it inside the pill and open a ring between fill and border. */}
      {pulse && !reduce ? (
        <motion.span
          aria-hidden
          className="absolute inset-0 rounded-full bg-current"
          initial={{ opacity: 0.06 }}
          animate={{ opacity: [0.06, 0.18, 0.06] }}
          transition={{ duration: 1.6, repeat: Infinity, ease: "easeInOut" }}
        />
      ) : null}
      {/* The badge animates its width with a scale transform. Slots opt into
          position-only layout so motion counter-scales them and the glyphs
          keep their shape while the pill stretches around them. */}
      {showIcon ? (
        <motion.span
          layout={slotLayout}
          transition={SPRING_LAYOUT}
          className="relative z-10 inline-grid shrink-0 place-items-center overflow-hidden"
        >
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={status}
              aria-hidden
              data-badge-icon
              variants={ICON_ROLL_VARIANTS}
              initial={reduce ? false : "initial"}
              animate={reduce ? REST : "animate"}
              exit={reduce ? REST_EXIT : "exit"}
              className="col-start-1 row-start-1 inline-flex items-center justify-center"
            >
              {status === "loading" && !reduce && !icon ? (
                <motion.span
                  animate={{ rotate: 360 }}
                  transition={{ duration: 1, repeat: Infinity, ease: "linear" }}
                  className="inline-flex"
                >
                  <Icon className={ICON_CLASS[size]} />
                </motion.span>
              ) : (
                (icon ?? <Icon className={ICON_CLASS[size]} />)
              )}
            </motion.span>
          </AnimatePresence>
        </motion.span>
      ) : null}
      {hasLabel ? (
        // Clip the roll to the line box only; a longer leaving label may run
        // past the slot and is clipped by the pill instead of cut mid-glyph.
        <motion.span
          layout={slotLayout}
          transition={SPRING_LAYOUT}
          className="relative z-10 inline-flex"
          style={{ clipPath: "inset(0 -999px)" }}
        >
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={resolvedContentKey}
              data-badge-label
              variants={TEXT_ROLL_VARIANTS}
              initial={reduce ? false : "initial"}
              animate={reduce ? REST : "animate"}
              exit={reduce ? REST_EXIT : "exit"}
              className="inline-block"
            >
              {children}
            </motion.span>
          </AnimatePresence>
        </motion.span>
      ) : null}
    </motion.span>
  );
}
