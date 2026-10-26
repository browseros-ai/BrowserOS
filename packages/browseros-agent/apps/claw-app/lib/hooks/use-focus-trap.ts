"use client";

import { type RefObject, useEffect, useLayoutEffect, useRef } from "react";

const FOCUSABLE =
  'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/** Elements inside `container` that Tab can reach, in DOM order. */
export function focusablesIn(container: Element | null) {
  return Array.from(
    container?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [],
  ).filter((element) => element.tabIndex >= 0 && !element.closest("[inert]"));
}

/**
 * Open traps, innermost last. Only the innermost one answers Tab, Escape and
 * stray focus: a sheet opened from inside a drawer portals outside the drawer,
 * and a drawer that still pulled focus home would fight the sheet for it.
 */
const traps: object[] = [];

export interface FocusTrapOptions {
  /** Called on Escape while this is the innermost open trap. */
  onEscape?: () => void;
}

/**
 * Moves focus into `panel` when `open` turns true, keeps Tab cycling inside it,
 * pulls focus back if it lands outside, and returns focus to whatever held it
 * before opening once `open` turns false.
 *
 * For overlays rendered in place, where `useModalScope` cannot inert the rest
 * of the page. Pass `open` rather than presence: an exiting panel is already
 * inert through `PresenceGate`, so the trap releases with the same commit.
 * Give the panel `tabIndex={-1}` so it can take focus when it holds nothing
 * focusable.
 */
export function useFocusTrap(
  open: boolean,
  panel: RefObject<HTMLElement | null>,
  { onEscape }: FocusTrapOptions = {},
) {
  const escapeRef = useRef(onEscape);
  useLayoutEffect(() => {
    escapeRef.current = onEscape;
  });

  useEffect(() => {
    if (!open || !panel.current) return;
    const self = {};
    traps.push(self);
    const innermost = () => traps[traps.length - 1] === self;
    const returnFocus =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const focusables = () => focusablesIn(panel.current);    const focusFirst = () =>
      (focusables()[0] ?? panel.current)?.focus({ preventScroll: true });
    if (!panel.current.contains(document.activeElement)) focusFirst();

    const onFocus = (event: FocusEvent) => {
      if (!innermost() || !panel.current || panel.current.closest("[inert]"))
        return;
      if (event.target instanceof Node && !panel.current.contains(event.target))
        focusFirst();
    };
    const onKey = (event: KeyboardEvent) => {
      if (!innermost() || event.defaultPrevented) return;
      if (event.key === "Escape" && escapeRef.current) {
        event.preventDefault();
        escapeRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const elements = focusables();
      const first = elements[0];
      const last = elements[elements.length - 1];
      const active = document.activeElement;
      if (!first || !last) {
        event.preventDefault();
        panel.current?.focus({ preventScroll: true });
      } else if (event.shiftKey && (active === first || active === panel.current)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("focusin", onFocus);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("focusin", onFocus);
      document.removeEventListener("keydown", onKey);
      traps.splice(traps.indexOf(self), 1);
      if (returnFocus?.isConnected && !returnFocus.closest("[inert]"))
        returnFocus.focus({ preventScroll: true });
    };
  }, [open, panel]);
}
