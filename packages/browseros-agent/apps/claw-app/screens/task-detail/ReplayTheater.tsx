import type { SessionSummary } from '@browseros/claw-api'
import { X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import {
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
} from 'react'
import { createPortal } from 'react-dom'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
import { Button } from '@/components/motion/button/base'
import { EASE_OUT } from '@/lib/ease'
import { useModalScope } from '@/lib/hooks/use-modal-scope'
import { PresenceGate } from '@/lib/presence-gate'
import { taskDayAndTime, tookLabel } from '@/screens/audit/audit.helpers'
import { ReplayPanel } from '@/screens/replay/ReplayPanel'
import type { ReplaySession } from '@/screens/replay/use-replay-session'

interface ReplayTheaterProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  session: SessionSummary
  /** The page's own replay session, so both players run on one clock. */
  replay: ReplaySession
  /** The page's step list, laid out as a rail. */
  steps: ReactNode
}

// Anything that takes typing, and other dialogs, keep F for themselves.
const KEEPS_F =
  'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="dialog"]'

/**
 * The task's recording at full size with the steps beside it. It plays the
 * page's own replay session rather than a copy, so time, speed and play state
 * are the same whichever player is showing. F opens it; Esc closes it and
 * focus goes back to whatever opened it.
 */
export function ReplayTheater({
  open,
  onOpenChange,
  session,
  replay,
  steps,
}: ReplayTheaterProps) {
  const reduce = useReducedMotion() ?? false
  const portal = useRef<HTMLDivElement>(null)
  const panel = useRef<HTMLDivElement>(null)
  const titleId = useId()
  const close = useCallback(() => onOpenChange(false), [onOpenChange])
  useModalScope(open, portal, panel, close)
  useOpenOnF(!open, () => onOpenChange(true))

  return createPortal(
    <div ref={portal}>
      <AnimatePresence>
        {open ? (
          <PresenceGate>
            {({ isPresent, gate }) => (
              <>
                <motion.button
                  {...gate}
                  type="button"
                  tabIndex={-1}
                  aria-hidden="true"
                  onClick={close}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: reduce ? 0.1 : 0.2, ease: EASE_OUT }}
                  className="fixed inset-0 z-[100] h-full w-full cursor-default bg-cyanotype-ink/85"
                />
                <div
                  inert={!isPresent}
                  className="pointer-events-none fixed inset-4 z-[100] flex sm:inset-x-10 sm:inset-y-[30px]"
                >
                  <motion.div
                    {...gate}
                    ref={panel}
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby={titleId}
                    tabIndex={-1}
                    // No scale: the replay fits itself to its on-screen box only
                    // when that box resizes, so a scale-in would leave it small.
                    initial={{ opacity: 0, y: reduce ? 0 : 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: reduce ? 0 : 8 }}
                    transition={{
                      duration: reduce ? 0.1 : 0.22,
                      ease: EASE_OUT,
                    }}
                    // Portaled outside the task page's ph-no-capture wrapper,
                    // so it blocks its own private content from recordings.
                    className="ph-no-capture flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl bg-card shadow-[0_30px_80px_-20px_rgb(1_18_63/0.4)] outline-none"
                  >
                    <header className="flex items-center justify-between gap-4 border-border-2 border-b pt-4 pr-5 pb-3.5 pl-6">
                      <div className="flex min-w-0 items-center gap-3">
                        <SessionStatusBadge
                          status={session.status}
                          className="shrink-0"
                        />
                        <h2
                          id={titleId}
                          className="truncate font-bold text-[17px] text-cyanotype-ink leading-[22px]"
                        >
                          {session.name}
                        </h2>
                        <p className="hidden shrink-0 text-[14px] text-ink-3 leading-[18px] md:block">
                          {session.label || session.slug} ·{' '}
                          {taskDayAndTime(session.startedAt, Date.now())} ·{' '}
                          {tookLabel(session.durationMs)}
                        </p>
                      </div>
                      <Button
                        size="icon"
                        variant="outline"
                        aria-label="Close full screen"
                        onClick={close}
                        className="size-[34px] shrink-0 border-border-2"
                      >
                        <X aria-hidden className="size-4" />
                      </Button>
                    </header>
                    <div className="grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)] lg:grid-cols-[minmax(0,1fr)_minmax(300px,480px)]">
                      <div className="flex min-h-0 min-w-0 flex-col px-6 py-5">
                        <ReplayPanel
                          session={replay}
                          fullScreen
                          onToggleFullScreen={close}
                        />
                      </div>
                      {/* The player is the point here; a narrow window keeps
                          it and leaves the steps to the page underneath. */}
                      <div className="hidden min-h-0 flex-col gap-1 border-border-2 border-l bg-bg-canvas p-5 lg:flex">
                        {steps}
                        <p className="px-2 text-[13px] text-ink-3 leading-[18px]">
                          Click a step to jump there. Esc to close.
                        </p>
                      </div>
                    </div>
                  </motion.div>
                </div>
              </>
            )}
          </PresenceGate>
        ) : null}
      </AnimatePresence>
    </div>,
    document.body,
  )
}

/** Opens on a bare F press anywhere on the page, the video-player shortcut. */
function useOpenOnF(enabled: boolean, onOpen: () => void) {
  const openRef = useRef(onOpen)
  useLayoutEffect(() => {
    openRef.current = onOpen
  })

  useEffect(() => {
    if (!enabled) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'f' && event.key !== 'F') return
      if (event.defaultPrevented || event.repeat) return
      if (event.metaKey || event.ctrlKey || event.altKey) return
      if (event.target instanceof Element && event.target.closest(KEEPS_F))
        return
      event.preventDefault()
      openRef.current()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [enabled])
}
