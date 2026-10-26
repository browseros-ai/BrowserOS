"use client";

import { type KeyboardEvent, type PointerEvent, useCallback, useRef, useState } from "react";
import { capturePointer, releasePointer } from "@/lib/touch";

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Nearest legal value on [min, max] for the given step. max counts as a
 * candidate when the step does not divide the range, so a pointer near the end
 * does not snap back onto the last whole step. */
export function snapSliderValue(next: number, min: number, max: number, step: number): number {
  // Neither case has a grid to walk. An empty range has exactly one legal
  // point, and a non-positive step only needs a clamp, which also keeps the
  // division below away from zero.
  if (!(max > min)) return min;
  if (!(step > 0)) return clamp(next, min, max);
  const whole = Math.floor(Number(((max - min) / step).toFixed(6)));
  const lastWhole = Number((min + whole * step).toFixed(6));
  const toGrid = clamp(Math.round((next - min) / step) * step + min, min, lastWhole);
  const snapped =
    lastWhole < max && Math.abs(next - max) <= Math.abs(next - toGrid) ? max : toGrid;
  return Number(snapped.toFixed(6));
}

export interface SliderOptions {
  value?: number;
  defaultValue?: number;
  onValueChange?: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  "aria-label"?: string;
  /** Announced instead of the raw number — pass one when the value carries a
   * unit or a suffix ("72.5 kg", "35%"); a bare number needs no valueText. */
  formatValueText?: (value: number) => string;
}

/**
 * Shared value + input plumbing for slider designs: controlled/uncontrolled
 * value, step snapping, pointer-capture drag along a track and arrow-key
 * control. Visuals and motion live in the component; this only owns the number.
 */
export function useSlider({
  value,
  defaultValue = 0,
  onValueChange,
  min = 0,
  max = 100,
  step = 1,
  disabled = false,
  "aria-label": ariaLabel,
  formatValueText,
}: SliderOptions) {
  const trackRef = useRef<HTMLDivElement>(null);
  const sliderEl = useRef<HTMLElement | null>(null);
  // The state drives visuals. Move reads this ref instead, so the first
  // pointermove after pointerdown does not have to wait on a re-render.
  const draggingRef = useRef(false);
  const [internal, setInternal] = useState(defaultValue);
  const [dragging, setDragging] = useState(false);
  const controlled = value !== undefined;
  // Collapse inverted or empty ranges and non-positive steps here, so that
  // percent, ticks and the keyboard maths never divide by zero or walk a
  // NaN grid.
  const lo = min;
  const hi = max > min ? max : min;
  const stride = step > 0 ? step : 1;
  const current = clamp(controlled ? value : internal, lo, hi);
  const percent = hi > lo ? ((current - lo) / (hi - lo)) * 100 : 0;

  const commit = useCallback(
    (next: number) => {
      const clean = snapSliderValue(next, lo, hi, stride);
      if (!controlled) setInternal(clean);
      onValueChange?.(clean);
    },
    [controlled, onValueChange, lo, hi, stride],
  );

  const commitFromX = useCallback(
    (clientX: number) => {
      const rect = trackRef.current?.getBoundingClientRect();
      if (!rect || rect.width === 0) return;
      const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
      commit(lo + ratio * (hi - lo));
    },
    [commit, lo, hi],
  );

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (disabled) return;
      // Start the drag first: capture is a convenience, and a browser that
      // refuses it — or a test DOM that has no pointer capture at all — must
      // not take the drag down with it.
      draggingRef.current = true;
      setDragging(true);
      capturePointer(event.currentTarget, event.pointerId);
      // A click on the track should land keyboard focus on the handle.
      sliderEl.current?.focus({ preventScroll: true });
      commitFromX(event.clientX);
    },
    [disabled, commitFromX],
  );

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (!draggingRef.current || disabled) return;
      commitFromX(event.clientX);
    },
    [disabled, commitFromX],
  );

  const endDrag = useCallback((event: PointerEvent<HTMLDivElement>) => {
    releasePointer(event.currentTarget, event.pointerId);
    draggingRef.current = false;
    setDragging(false);
  }, []);

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLElement>) => {
      if (disabled) return;
      const map: Record<string, number> = {
        ArrowRight: current + stride,
        ArrowUp: current + stride,
        ArrowLeft: current - stride,
        ArrowDown: current - stride,
        PageUp: current + stride * 10,
        PageDown: current - stride * 10,
        Home: lo,
        End: hi,
      };
      if (event.key in map) {
        event.preventDefault();
        commit(map[event.key]);
      }
    },
    [disabled, current, stride, lo, hi, commit],
  );

  return {
    current,
    percent,
    dragging,
    min: lo,
    max: hi,
    step: stride,
    commit,
    /** Pointer handlers for the track element — drag anywhere on it. */
    trackProps: {
      ref: trackRef,
      onPointerDown,
      onPointerMove,
      onPointerUp: endDrag,
      onPointerCancel: endDrag,
      onLostPointerCapture: endDrag,
    },
    /** ARIA + keyboard props for the focusable slider element. */
    sliderProps: {
      // Callback keeps the handle typed across button/div/motion hosts.
      ref: (node: HTMLElement | null) => {
        sliderEl.current = node;
      },
      role: "slider" as const,
      tabIndex: disabled ? -1 : 0,
      "aria-label": ariaLabel,
      "aria-valuemin": lo,
      "aria-valuemax": hi,
      "aria-valuenow": current,
      "aria-valuetext": formatValueText?.(current),
      "aria-disabled": disabled || undefined,
      onKeyDown,
    },
  };
}
