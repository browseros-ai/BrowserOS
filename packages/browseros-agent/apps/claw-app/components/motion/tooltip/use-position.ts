"use client";

import {
  arrow as arrowMiddleware,
  autoUpdate,
  computePosition,
  flip,
  hide,
  offset,
  type Placement,
  shift,
  type VirtualElement,
} from "@floating-ui/dom";
import { type RefObject, useCallback, useLayoutEffect, useRef } from "react";

export type TooltipSide = "top" | "right" | "bottom" | "left";
export type TooltipAlign = "start" | "center" | "end";
export type TooltipPoint = { x: number; y: number };
export type TooltipAnchor = HTMLElement | SVGElement;

const ORIGIN: Record<TooltipSide, string> = {
  top: "center bottom",
  bottom: "center top",
  left: "right center",
  right: "left center",
};

/** Position is geometry, not animation. One write per frame, never a spring chasing a pointer. */
export function useTooltipPosition({
  open,
  anchor,
  floatingRef,
  arrowRef,
  anchorPoint,
  followCursor,
  side,
  align,
  sideOffset,
  onDismiss,
  onPosition,
  pointer,
}: {
  open: boolean;
  anchor: TooltipAnchor | null;
  floatingRef: RefObject<HTMLSpanElement | null>;
  arrowRef?: RefObject<SVGSVGElement | null>;
  anchorPoint?: TooltipPoint;
  followCursor: boolean;
  side: TooltipSide;
  align: TooltipAlign;
  sideOffset?: number;
  onDismiss: () => void;
  onPosition: (side: TooltipSide) => void;
  pointer: ReturnType<typeof useTooltipPointer>;
}) {
  const { cursor, onMove } = pointer;
  const preferred: Placement = align === "center" ? side : `${side}-${align}`;
  const cursorSide = useRef<Placement | null>(null);
  useLayoutEffect(() => {
    cursorSide.current = open ? preferred : null;
  }, [open, preferred]);
  const frame = useRef<number | null>(null);
  const version = useRef(0);
  const update = useRef<() => void>(() => {});
  const schedule = useCallback(() => {
    // Invalidate older async calculations as soon as new geometry is requested.
    version.current++;
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      update.current();
    });
  }, []);
  useLayoutEffect(() => {
    onMove.current = schedule;
    return () => {
      onMove.current = null;
    };
  }, [onMove, schedule]);
  const pointX = anchorPoint?.x;
  const pointY = anchorPoint?.y;

  // Latest committed inputs are read without recreating observers for every period/content update.
  useLayoutEffect(() => {
    update.current = () => {
      const floating = floatingRef.current;
      if (!open || !anchor || !floating) return;
      const revision = ++version.current;
      const currentCursor = followCursor ? cursor.current : null;
      const virtual = Boolean(currentCursor) || pointX !== undefined || pointY !== undefined;
      const reference: Element | VirtualElement = virtual
        ? {
            contextElement: anchor,
            getBoundingClientRect: () => {
              const rect = anchor.getBoundingClientRect();
              const x = currentCursor?.x ?? rect.left + rect.width * (pointX ?? 0.5);
              const y = currentCursor?.y ?? rect.top + rect.height * (pointY ?? 0.5);
              return { x, y, left: x, right: x, top: y, bottom: y, width: 0, height: 0 };
            },
          }
        : anchor;
      const arrowElement = arrowRef?.current ?? null;
      void computePosition(reference, floating, {
        strategy: "fixed",
        placement: currentCursor ? (cursorSide.current ?? preferred) : preferred,
        middleware: [
          offset(sideOffset ?? (currentCursor ? 12 : 8)),
          flip({ padding: 8 }),
          shift({ padding: 8 }),
          ...(arrowElement ? [arrowMiddleware({ element: arrowElement, padding: 10 })] : []),
          // A label left floating over whatever scrolled into its trigger's place
          // describes nothing there. Points inside a chart move with the chart.
          ...(virtual ? [] : [hide({ strategy: "referenceHidden" })]),
        ],
      }).then(({ x, y, placement, middlewareData }) => {
        if (version.current !== revision || !floating.isConnected) return;
        // Hold the chosen side for this hover session. Crossing a flip threshold
        // repeatedly must not bounce the surface above and below the pointer.
        if (currentCursor) cursorSide.current = placement;
        else cursorSide.current = null;
        const placedSide = placement.split("-")[0] as TooltipSide;
        const dpr = window.devicePixelRatio || 1;
        floating.style.transform = `translate3d(${Math.round(x * dpr) / dpr}px, ${Math.round(y * dpr) / dpr}px, 0)`;
        floating.style.setProperty("--tooltip-origin", ORIGIN[placedSide]);
        // A box with no size has no clipped state to report, only a missing layout.
        const box = anchor.getBoundingClientRect();
        const clipped =
          Boolean(middlewareData.hide?.referenceHidden) && (box.width > 0 || box.height > 0);
        floating.style.visibility = clipped ? "hidden" : "visible";
        floating.dataset.placement = placement;
        const surface = arrowElement?.parentElement;
        if (arrowElement && surface) {
          // Measured against the positioner, drawn inside the surface's border.
          const { x: ax, y: ay } = middlewareData.arrow ?? {};
          if (ax != null) arrowElement.style.left = `${ax - surface.clientLeft}px`;
          if (ay != null) arrowElement.style.top = `${ay - surface.clientTop}px`;
          // Paint with whatever the surface resolved to, so a restyled tooltip
          // keeps a matching arrow without a second set of classes.
          const style = getComputedStyle(surface);
          arrowElement.style.setProperty("--tooltip-arrow-fill", style.backgroundColor);
          arrowElement.style.setProperty("--tooltip-arrow-stroke", style.borderTopColor);
        }
        onPosition(placedSide);
      });
    };
    if (open) schedule();
  }, [
    open,
    anchor,
    floatingRef,
    arrowRef,
    followCursor,
    pointX,
    pointY,
    preferred,
    sideOffset,
    schedule,
    onPosition,
    cursor,
  ]);

  useLayoutEffect(() => {
    const floating = floatingRef.current;
    if (!open || !anchor || !floating) return;
    const stop = autoUpdate(anchor, floating, schedule);
    const onScroll = () => {
      // A stationary pointer no longer describes the same chart point after scrolling.
      if (followCursor && cursor.current) onDismiss();
    };
    window.addEventListener("scroll", onScroll, true);
    return () => {
      stop();
      window.removeEventListener("scroll", onScroll, true);
      version.current++;
      if (frame.current !== null) cancelAnimationFrame(frame.current);
      frame.current = null;
    };
  }, [open, anchor, floatingRef, followCursor, onDismiss, schedule, cursor]);

  useLayoutEffect(
    () => () => {
      version.current++;
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );
}

/** Pointer lifetime belongs to the anchor, including the opening delay. */
export function useTooltipPointer(anchor: TooltipAnchor | null, followCursor: boolean) {
  const cursor = useRef<TooltipPoint | null>(null);
  const onMove = useRef<(() => void) | null>(null);
  const pointerFocus = useRef(false);
  useLayoutEffect(() => {
    if (!anchor || !followCursor) return;
    const point = (event: PointerEvent) => {
      if (event.type === "pointermove" && event.pointerType === "touch" && !event.buttons) return;
      cursor.current = { x: event.clientX, y: event.clientY };
      if (event.type === "pointerdown") pointerFocus.current = true;
      onMove.current?.();
    };
    const keyboard = () => {
      cursor.current = null;
      pointerFocus.current = false;
      onMove.current?.();
    };
    const focus = () => {
      if (!pointerFocus.current) cursor.current = null;
      pointerFocus.current = false;
      onMove.current?.();
    };
    const leave = () => {
      cursor.current = null;
    };
    anchor.addEventListener("pointerenter", point as EventListener, { passive: true });
    anchor.addEventListener("pointermove", point as EventListener, { passive: true });
    anchor.addEventListener("pointerdown", point as EventListener, { passive: true });
    anchor.addEventListener("pointerleave", leave);
    anchor.addEventListener("pointercancel", leave);
    anchor.addEventListener("keydown", keyboard);
    anchor.addEventListener("focusin", focus);
    return () => {
      cursor.current = null;
      anchor.removeEventListener("pointerenter", point as EventListener);
      anchor.removeEventListener("pointermove", point as EventListener);
      anchor.removeEventListener("pointerdown", point as EventListener);
      anchor.removeEventListener("pointerleave", leave);
      anchor.removeEventListener("pointercancel", leave);
      anchor.removeEventListener("keydown", keyboard);
      anchor.removeEventListener("focusin", focus);
    };
  }, [anchor, followCursor]);

  return { cursor, onMove };
}
