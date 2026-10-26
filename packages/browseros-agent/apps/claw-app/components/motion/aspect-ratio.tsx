"use client";
// beui.dev/components/motion/aspect-ratio

import { type HTMLMotionProps, motion, useReducedMotion } from "motion/react";
import { createContext, type ReactNode, useContext, useMemo } from "react";
import { SPRING_LAYOUT } from "@/lib/ease";
import { cn } from "@/lib/utils";

type AspectRatioContextValue = { ratio: number; animated: boolean };
const AspectRatioContext = createContext<AspectRatioContextValue | null>(null);

export interface AspectRatioProps
  extends Omit<HTMLMotionProps<"div">, "children" | "layout" | "layoutDependency"> {
  children?: ReactNode;
  /** Width divided by height, for example 16 / 9. Must be finite and positive. */
  ratio?: number;
  /** Morph layout changes. Reduced motion always skips movement. */
  animated?: boolean;
  /** Classes for the content layer, such as alignment or padding. */
  contentClassName?: string;
}

export function AspectRatio({
  ratio = 1,
  animated = true,
  children,
  className,
  contentClassName,
  style,
  ...props
}: AspectRatioProps) {
  const reduce = useReducedMotion();
  const enabled = animated && !reduce;
  const context = useMemo(() => ({ ratio, animated: enabled }), [ratio, enabled]);

  if (!Number.isFinite(ratio) || ratio <= 0) {
    throw new RangeError("AspectRatio: ratio must be a finite, positive number.");
  }

  return (
    <AspectRatioContext.Provider value={context}>
      <motion.div
        {...props}
        layout={enabled}
        transition={{ layout: SPRING_LAYOUT }}
        className={cn("relative w-full min-w-0 overflow-hidden", className)}
        style={{ ...style, aspectRatio: ratio }}
      >
        <div className={cn("absolute inset-0", contentClassName)}>{children}</div>
      </motion.div>
    </AspectRatioContext.Provider>
  );
}

export interface AspectRatioImageProps
  extends Omit<HTMLMotionProps<"img">, "layout" | "layoutDependency" | "width" | "height"> {
  alt: string;
  /** Intrinsic width in pixels, used to reserve the image's cover crop. */
  width: number;
  /** Intrinsic height in pixels. Keep these dimensions accurate for the source. */
  height: number;
}

export function AspectRatioImage({
  width,
  height,
  className,
  style,
  draggable = false,
  ...props
}: AspectRatioImageProps) {
  const context = useContext(AspectRatioContext);
  if (!context) throw new Error("AspectRatioImage must be inside AspectRatio.");
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    throw new RangeError("AspectRatioImage: width and height must be finite, positive numbers.");
  }
  const fillWidth = context.ratio >= width / height;

  return (
    <div className="absolute inset-0 grid grid-cols-1 grid-rows-1 place-items-center">
      {/* The image box keeps its intrinsic ratio at both endpoints. Its layout
          projection therefore scales uniformly while the frame reshapes. */}
      {/* biome-ignore lint/performance/noImgElement: Copy-paste registry components work outside Next.js. */}
      <motion.img
        {...props}
        width={width}
        height={height}
        draggable={draggable}
        layout={context.animated}
        transition={{ layout: SPRING_LAYOUT }}
        className={cn("col-start-1 row-start-1 block max-w-none", className)}
        style={{
          ...style,
          width: fillWidth ? "100%" : "auto",
          height: fillWidth ? "auto" : "100%",
        }}
      />
    </div>
  );
}
