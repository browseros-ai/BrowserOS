"use client";

import { useMemo, useRef } from "react";
import { isHoveringPointer } from "@/lib/touch";

interface BoundaryEvent {
  pointerId: number;
  pointerType: string;
  buttons: number;
}

export interface HoverGesture {
  /** True when this enter starts a hover: the pointer arrived resting, not pressing. */
  enter: (event: BoundaryEvent) => boolean;
  /** True when this leave ends a hover that entered as one. */
  leave: (event: BoundaryEvent) => boolean;
}

/**
 * Pairs a surface's enter with its leave, per pointer.
 *
 * `isHoveringPointer` answers the question the *enter* asks — is this pointer
 * resting on the surface or pressing it — and both boundary cases go wrong if
 * the leave is asked the same question again:
 *
 * - A pen with no hover never rests. It arrives in contact, taps, and the spec
 *   then requires its boundary events after `pointerup`, so the leave carries
 *   `buttons: 0` and reads as a mouse gliding off. Hover teardown then undid
 *   the tap — the panel the pen had just opened closed under it.
 * - A mouse pressed on the surface and dragged off leaves with `buttons: 1`.
 *   Skipping teardown there strands the surface open: the release happens
 *   outside, and no second leave ever comes.
 *
 * So the state a hover holds is released by the pointer that took it, whatever
 * the buttons say at the boundary, and a pointer that arrived in contact never
 * took it in the first place. Contact is the exception tracked here, not
 * hover: a leave from a pointer this surface never saw enter — mounted under
 * the cursor, say — still counts, since the alternative is state with no way
 * out.
 */
export function useHoverGesture(): HoverGesture {
  const contact = useRef(new Set<number>());

  return useMemo(
    () => ({
      enter: (event) => {
        if (isHoveringPointer(event)) {
          contact.current.delete(event.pointerId);
          return true;
        }
        contact.current.add(event.pointerId);
        return false;
      },
      leave: (event) => {
        const arrivedInContact = contact.current.delete(event.pointerId);
        return !arrivedInContact && event.pointerType !== "touch";
      },
    }),
    [],
  );
}
