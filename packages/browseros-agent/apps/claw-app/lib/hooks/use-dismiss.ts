"use client";

import { type RefObject, useEffect } from "react";

/**
 * What the dismissing gesture does to the control it landed on.
 *
 * `"pass-through"` is the platform norm (native popover light-dismiss): the
 * tap closes the overlay *and* activates whatever was under it. Use
 * `"consume"` where the open overlay sits over or beside controls that would
 * be costly to trigger by accident — the dismissal then swallows the
 * activation too, so the gesture only closes.
 */
export type DismissBehavior = "pass-through" | "consume";

export interface DismissOptions {
  /** Default `"pass-through"`. */
  behavior?: DismissBehavior;
  /** Dismiss on Escape as well. Default true. */
  escape?: boolean;
  /** Return true for an outside target that should *not* dismiss. Must be stable. */
  ignore?: (target: Element) => boolean;
}

/**
 * What every currently open dismiss scope counts as inside itself. A consumed
 * dismissal reads this to tell a stray gesture from one that belongs to an
 * overlay in front of it: overlays have no shared z-order to consult, but the
 * one the gesture landed in has said as much by registering it.
 */
const openScopes = new Set<(target: Element) => boolean>();

function claimedByAnotherScope(
  self: (target: Element) => boolean,
  target: Element,
) {
  for (const scope of openScopes) {
    if (scope !== self && scope(target)) return true;
  }
  return false;
}

// preventDefault on pointerdown does not suppress the click that follows, so
// consuming a gesture means swallowing that click itself. The swallower
// deliberately outlives the effect that installed it — the dismissal it
// belongs to has already unmounted or re-rendered by the time the click lands.
// It releases on that click, or on the next gesture if the pointer is dragged
// away and no click ever arrives, so it can never eat a later one. A keydown
// releases it too: a gesture that ends with neither a click nor a cancel would
// otherwise leave it armed, and the click Enter synthesizes on some focused
// control is not the one this dismissal was owed.
function consumeActivation(source: Event) {
  const swallow = (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    release();
  };
  const restart = (event: Event) => {
    if (event !== source) release();
  };
  const release = () => {
    window.removeEventListener("click", swallow, true);
    window.removeEventListener("pointerdown", restart, true);
    window.removeEventListener("pointercancel", restart, true);
    window.removeEventListener("keydown", release, true);
  };
  window.addEventListener("click", swallow, true);
  window.addEventListener("pointerdown", restart, true);
  window.addEventListener("pointercancel", restart, true);
  window.addEventListener("keydown", release, true);
}

/**
 * Close an open overlay on Escape or a pointerdown outside `ref`. Pass `null`
 * for `ref` when what counts as inside isn't one element, and say so with
 * `ignore` instead.
 *
 * The pointerdown listener is capture-phase: a bubble-phase one is blinded by
 * any handler in between that stops propagation, and an overlay cannot know
 * what it is layered over. `onDismiss` and `ignore` must be stable (wrap in
 * useCallback) so the listeners aren't re-bound every render while open.
 */
export function useDismiss(
  open: boolean,
  onDismiss: () => void,
  ref: RefObject<HTMLElement | SVGElement | null> | null,
  {
    behavior = "pass-through",
    escape: dismissOnEscape = true,
    ignore,
  }: DismissOptions = {},
) {
  useEffect(() => {
    if (!open) return;
    const inside = (target: Element) =>
      Boolean(ref?.current?.contains(target)) || Boolean(ignore?.(target));
    const onKey = (event: KeyboardEvent) => {
      if (dismissOnEscape && event.key === "Escape") onDismiss();
    };
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (!target || inside(target)) return;
      // Outside this overlay, but inside one that is also open: the gesture is
      // that overlay's to answer, and swallowing its click from behind would
      // cost the user the control they actually aimed at.
      if (behavior === "consume" && !claimedByAnotherScope(inside, target)) {
        consumeActivation(event);
      }
      onDismiss();
    };
    openScopes.add(inside);
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onPointer, true);
    return () => {
      openScopes.delete(inside);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onPointer, true);
    };
  }, [open, onDismiss, ref, behavior, dismissOnEscape, ignore]);
}
