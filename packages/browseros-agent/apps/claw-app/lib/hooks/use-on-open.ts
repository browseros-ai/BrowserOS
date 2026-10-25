"use client";

import { useState } from "react";

/**
 * Runs `start` during the render in which `open` becomes true.
 *
 * Opening a list starts a fresh session — an empty query, the highlight back at
 * the top — and that is a resolution, not a side effect: a passive effect runs
 * after the commit, so the new session would carry the last one's state for a
 * window in which a key press can land.
 *
 * `start` may only set state belonging to the calling component. Anything that
 * reaches outside it — a consumer's callback, a DOM write, moving focus — is a
 * side effect, and React rejects a callback that sets another component's state
 * during this one's render. Put those in an effect keyed to `open`.
 */
export function useOnOpen(open: boolean, start: () => void) {
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) start();
  }
}
