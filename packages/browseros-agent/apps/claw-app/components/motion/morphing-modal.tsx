"use client";
// beui.dev/components/motion/morphing-modal

import {
  AnimatePresence,
  motion,
  useReducedMotion,
} from "motion/react";
import {
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { EASE_OUT, SPRING_PANEL } from "@/lib/ease";
import { focusablesIn, useFocusTrap } from "@/lib/hooks/use-focus-trap";
import { PresenceGate } from "@/lib/presence-gate";
import { cn } from "@/lib/utils";

export interface MorphingModalProps {
  /** Which view is currently shown. `null` closes the modal. */
  viewId: string | null;
  onClose: () => void;
  children: ReactNode;
  /** "bottom" anchors to the viewport bottom (mobile-like). "center" centers vertically. */
  placement?: "bottom" | "center";
  /**
   * Accessible name for the dialog. Without it, the dialog is named by the
   * first heading in the current view.
   */
  ariaLabel?: string;
  className?: string;
}

export function MorphingModal({
  viewId,
  onClose,
  children,
  placement = "bottom",
  ariaLabel,
  className,
}: MorphingModalProps) {
  const open = viewId !== null;
  const reduce = useReducedMotion();
  const panelRef = useRef<HTMLDivElement>(null);
  const uid = useId();
  const [headingId, setHeadingId] = useState<string>();

  // Focus moves into the panel on open, Tab stays inside it, Escape closes,
  // and focus returns to the opener on close.
  useFocusTrap(open, panelRef, { onEscape: onClose });

  // The current view, not the one still fading out under it.
  const currentView = () =>
    Array.from(
      panelRef.current?.querySelectorAll<HTMLElement>("[data-view]") ?? [],
    ).find((element) => element.dataset.view === viewId);

  // Name the dialog by the current view's first heading unless a label is set.
  useLayoutEffect(() => {
    if (!viewId || ariaLabel) return;
    const heading = currentView()?.querySelector<HTMLElement>(
      "h1, h2, h3, h4, h5, h6",
    );
    if (heading && !heading.id) heading.id = `${uid}-${viewId}-title`;
    setHeadingId(heading?.id);
  });

  // The focused control leaves with the old view, so a view change hands focus
  // to the first control of the new one.
  const shownView = useRef(viewId);
  useEffect(() => {
    const previous = shownView.current;
    shownView.current = viewId;
    if (!viewId || !previous || previous === viewId) return;
    const panel = panelRef.current;
    const active = document.activeElement;
    if (!panel || (active && active !== document.body && !panel.contains(active)))
      return;
    const view = currentView();
    (focusablesIn(view ?? null)[0] ?? panel).focus({ preventScroll: true });
  });
  const enterY = reduce ? 0 : placement === "bottom" ? 40 : 20;
  const enterScale = reduce ? 1 : 0.97;

  useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [open]);

  // Mounted only while open, and while open the chrome is two fixed siblings
  // rather than one wrapper: the backdrop spans the viewport edges but carries
  // the scrim colour, and the layer positioning the panel sits inset off every
  // edge (`inset-4`, with the bottom placement's `pb-4` on top of it). Both hang
  // off `PresenceGate`, so interaction releases in the same commit that starts
  // the exit rather than when it ends — `open` is already false for those
  // frames.
  return (
    <AnimatePresence initial={false}>
      {open ? (
        <PresenceGate key="backdrop">
          {({ gate }) => (
            <motion.button
              type="button"
              aria-label="Close modal"
              // Outside the panel, so the focus trap never lands on it; it is
              // a pointer target, and Escape closes from the keyboard.
              tabIndex={-1}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.2, ease: EASE_OUT }}
              {...gate}
              onClick={onClose}
              className="pointer-events-auto fixed inset-0 z-[80] bg-background/5 [backdrop-filter:blur(14px)_saturate(140%)] [-webkit-backdrop-filter:blur(14px)_saturate(140%)]"
            />
          )}
        </PresenceGate>
      ) : null}

      {open ? (
        <PresenceGate key="panel-layer">
          {({ isPresent, gate }) => (
            // The layer itself never takes pointer events, so it carries
            // `inert` alone rather than the gate's pointer-events value.
            <div
              inert={!isPresent}
              className={cn(
                "pointer-events-none fixed inset-4 z-[80] flex justify-center",
                placement === "bottom" ? "items-end pb-4" : "items-center",
              )}
            >
              <motion.div
                key="panel"
                ref={panelRef}
                role="dialog"
                aria-modal="true"
                aria-label={ariaLabel}
                aria-labelledby={ariaLabel ? undefined : headingId}
                tabIndex={-1}
                layout
                initial={{ opacity: 0, y: enterY, scale: enterScale }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{
                  opacity: 0,
                  y: enterY,
                  scale: reduce ? 1 : 0.98,
                  transition: { duration: 0.18, ease: EASE_OUT },
                }}
                transition={SPRING_PANEL}
                {...gate}
                className={cn(
                  "pointer-events-auto relative w-full max-w-sm overflow-hidden rounded-3xl border border-border bg-background shadow-2xl outline-none will-change-transform",
                  className,
                )}
              >
                <motion.div layout="position" className="p-5">
                  <AnimatePresence mode="popLayout" initial={false}>
                    <PresenceGate key={viewId}>
                      {({ isPresent: viewPresent }) => (
                    <motion.div
                      // The outgoing view stays for its exit; inert keeps the
                      // trap and assistive technology on the incoming one.
                      inert={!viewPresent}
                      data-view={viewId}
                      initial={
                        reduce
                          ? { opacity: 0 }
                          : { opacity: 0, y: 8, filter: "blur(4px)" }
                      }
                      animate={
                        reduce
                          ? {
                              opacity: 1,
                              transition: {
                                duration: 0.18,
                                ease: EASE_OUT,
                              },
                            }
                          : {
                              opacity: 1,
                              y: 0,
                              filter: "blur(0px)",
                              transition: {
                                duration: 0.24,
                                ease: EASE_OUT,
                              },
                            }
                      }
                      exit={
                        reduce
                          ? {
                              opacity: 0,
                              transition: {
                                duration: 0.14,
                                ease: EASE_OUT,
                              },
                            }
                          : {
                              opacity: 0,
                              y: -8,
                              filter: "blur(4px)",
                              transition: {
                                duration: 0.16,
                                ease: EASE_OUT,
                              },
                            }
                      }
                    >
                      {children}
                    </motion.div>
                      )}
                    </PresenceGate>
                  </AnimatePresence>
                </motion.div>
              </motion.div>
            </div>
          )}
        </PresenceGate>
      ) : null}
    </AnimatePresence>
  );
}
