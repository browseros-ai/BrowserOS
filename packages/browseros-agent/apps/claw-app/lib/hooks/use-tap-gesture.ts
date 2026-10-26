"use client";

import { useMemo, useRef } from "react";

/** What a pointerdown recorded, read back by the click that ends its gesture. */
export interface TapRecord<S> {
  /** Which input started the gesture. */
  pointerType: string;
  /** What the surface was showing when it started. */
  state: S;
}

export interface TapGesture<S> {
  /** Record the gesture a pointerdown starts, with the state it starts in. */
  start: (event: { pointerType: string }, state: S) => void;
  /** Read the record and clear it. `null` when no pointer is behind this click. */
  take: () => TapRecord<S> | null;
  /** Drop the record: this gesture will never spend it on a click. */
  drop: () => void;
}

/**
 * The pointer gesture behind a click, recorded where the click cannot report
 * it. A `click` carries no `pointerType` in the engines that matter, so the
 * `pointerdown` before it is the only thing that says which input activated
 * the control — and whether one did at all, since keyboard activation
 * synthesizes a click with no pointer behind it.
 *
 * State goes in with the record because a click reports that no better: a
 * browser that focuses a control on contact can open the very panel the tap
 * was meant to open, and reading "is it open" at click time then undoes it.
 * What the gesture started against is what it acts on.
 *
 * The record is spent by one click and dropped by everything else, because a
 * record that outlives its gesture is worse than none:
 *
 * - A scroll or an OS gesture takes the touch away — `pointercancel`, no click
 *   ever — and the finger would sit in the record until some later click.
 * - That later click is often `Enter` on a keyboard, which arrives with no
 *   pointerdown of its own and would inherit the abandoned finger. A keydown
 *   is the start of a keyboard activation and never part of a tap, so it drops
 *   the record too.
 *
 * Both ends have to be wired by the surface: `drop` on `onPointerCancel` and
 * on `onKeyDown`.
 */
export function useTapGesture<S>(): TapGesture<S> {
  const record = useRef<TapRecord<S> | null>(null);

  return useMemo(
    () => ({
      start: (event, state) => {
        record.current = { pointerType: event.pointerType, state };
      },
      take: () => {
        const spent = record.current;
        record.current = null;
        return spent;
      },
      drop: () => {
        record.current = null;
      },
    }),
    [],
  );
}
