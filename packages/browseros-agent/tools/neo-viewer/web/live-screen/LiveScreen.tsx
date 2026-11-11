/**
 * The live-screen panel, self-contained so any page can place it: `useLiveScreen(task)` makes
 * the controller, and these take it.
 *
 * - `<LiveScreen size="docked">`: the sidebar panel ("Viewer C v2: Run blocks with their
 *   methods"): Follow agent / Control and Pop out over a small screen. While popped out it
 *   shows where the screen went.
 * - `<LiveScreenPopup>`: the same panel at `size="large"` in a pop-up over the page
 *   ("Viewer C v2: Live screen popped out"), with Take over and a close button.
 * - `<TakeOverButton>`: Take over / Give back, for a top bar.
 *
 * Only one view connects at a time: the docked panel stays empty while popped out.
 */

import { type ReactNode, useEffect, useRef } from 'react'
import type { LiveTab } from '../../shared/live-screen'
import {
  modeLine,
  type ScreenMode,
  screenNotice,
  takeOverAvailability,
} from './live-screen-model'
import type { LiveScreenController } from './use-live-screen'
import { VncScreen } from './VncScreen'

const SANS = "font-['IBM_Plex_Sans',system-ui,sans-serif]"
const MONO = "font-['JetBrains_Mono',ui-monospace,monospace]"

export type LiveScreenSize = 'docked' | 'large'

export function LiveScreen({
  live,
  size,
  popped = false,
  onPopOut,
  onClose,
}: {
  live: LiveScreenController
  size: LiveScreenSize
  /** Docked: the screen is open in the pop-up, so this one does not connect. */
  popped?: boolean
  /** Docked: shows Pop out. */
  onPopOut?: () => void
  /** Large: shows the close button. */
  onClose?: () => void
}) {
  const large = size === 'large'
  const { state } = live
  const screen = popped ? (
    <div className="flex h-[200px] items-center justify-center rounded-[6px] border border-[#C9D2E0] border-dashed bg-[#F8F9FB] text-[#4A5365] text-[12px] leading-4">
      Open in a pop-up. Close it to dock it here again.
    </div>
  ) : (
    <div
      className={
        large
          ? 'relative h-[640px] max-h-[calc(100vh-200px)] w-full overflow-clip bg-white'
          : 'relative h-[200px] w-full overflow-clip rounded-[6px] border border-[#0D1730]'
      }
    >
      <VncScreen
        display={live.display}
        attempt={state.attempt}
        onConnected={live.connected}
        onClosed={live.closed}
      />
      <Notice live={live} large={large} />
    </div>
  )

  if (!large)
    return (
      <section
        className={`flex flex-col gap-2 border-[#E4E7EC] border-b pb-3 text-[12px] leading-4 ${SANS}`}
        aria-label="Live screen"
      >
        <div className="flex items-center gap-2">
          <h2 className="font-semibold text-[#0D1730] text-[13px] leading-4">
            Live screen
          </h2>
          <div className="flex-1" />
          <ModeSwitch live={live} large={false} />
          {onPopOut && (
            <button
              type="button"
              onClick={onPopOut}
              className="flex items-center gap-[5px] rounded-[5px] border border-[#D5D9E0] px-2 py-0.5 font-medium text-[#0D1730] text-[11px] leading-[14px] hover:bg-[#F8F9FB]"
            >
              <PopOutIcon />
              Pop out
            </button>
          )}
        </div>
        {screen}
        <p className="text-[#4A5365] text-[11px] leading-[14px]">
          {modeLine(state, live.task)}
        </p>
        <ActionError live={live} />
      </section>
    )

  return (
    <section
      className={`flex flex-col text-[12px] leading-4 ${SANS}`}
      aria-label="Live screen"
    >
      <header className="flex h-[54px] shrink-0 items-center gap-3 border-[#E4E7EC] border-b pr-4 pl-5">
        <h2 className="font-semibold text-[#0D1730] text-[16px] leading-5">
          Live screen
        </h2>
        {state.status?.tab && <TabChip tab={state.status.tab} />}
        <div className="flex-1" />
        <ModeSwitch live={live} large />
        <TakeOverButton live={live} />
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            aria-label="Close and dock the screen"
            className="rounded p-1 hover:bg-[#F1F3F6]"
          >
            <CloseIcon />
          </button>
        )}
      </header>
      {screen}
      <footer className="flex h-10 shrink-0 items-center gap-3 border-[#E4E7EC] border-t px-5 text-[#4A5365]">
        <span className={state.takeover ? 'font-medium text-[#1F4FD8]' : ''}>
          {modeLine(state, live.task)}
        </span>
        <div className="flex-1" />
        <ActionError live={live} />
        {onClose && <span>Close to dock it back in the sidebar</span>}
      </footer>
    </section>
  )
}

/** The large panel in a pop-up over the page. Escape or the close button docks it again. */
export function LiveScreenPopup({
  live,
  onClose,
}: {
  live: LiveScreenController
  onClose: () => void
}) {
  const panel = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // Keys typed into the screen in control mode are the browser's, not the pop-up's.
      const screen = panel.current?.querySelector('[data-display]')
      if (event.key !== 'Escape' || screen?.contains(event.target as Node))
        return
      onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-[#0D173052] p-8">
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label="Live screen"
        className="w-[1100px] max-w-full overflow-clip rounded-[10px] bg-white shadow-[0_24px_60px_#0D173047]"
      >
        <LiveScreen live={live} size="large" onClose={onClose} />
      </div>
    </div>
  )
}

export function TakeOverButton({ live }: { live: LiveScreenController }) {
  const action = takeOverAvailability(live.state, live.task)
  const giving = action.label === 'Give back'
  return (
    <button
      type="button"
      disabled={!action.enabled}
      title={action.hint}
      onClick={giving ? live.giveBack : live.takeOver}
      className={
        giving
          ? 'shrink-0 rounded-[6px] border border-[#1F4FD8] bg-[#1F4FD8] px-2.5 py-[5px] font-medium text-[12px] text-white leading-4 hover:bg-[#1A43B8] disabled:opacity-60'
          : 'shrink-0 rounded-[6px] border border-[#D5D9E0] px-2.5 py-[5px] font-medium text-[#0D1730] text-[12px] leading-4 hover:bg-[#F8F9FB] disabled:cursor-not-allowed disabled:text-[#9AA1AE]'
      }
    >
      {live.state.busy
        ? giving
          ? 'Giving back…'
          : 'Taking over…'
        : action.label}
    </button>
  )
}

function ModeSwitch({
  live,
  large,
}: {
  live: LiveScreenController
  large: boolean
}) {
  const held = live.state.takeover !== null
  const current: ScreenMode = held ? 'control' : live.state.mode
  const option = (mode: ScreenMode, label: string): ReactNode => {
    const active = current === mode
    return (
      <button
        type="button"
        aria-pressed={active}
        disabled={held}
        title={
          held
            ? 'You have taken over: give the tab back to follow again.'
            : undefined
        }
        onClick={() => live.setMode(mode)}
        className={`${large ? 'px-2.5 py-1 text-[12px] leading-4' : 'px-2 py-0.5 text-[11px] leading-[14px]'} ${
          active
            ? 'bg-[#EEF3FD] font-semibold text-[#1F4FD8]'
            : 'text-[#0D1730] hover:bg-[#F8F9FB]'
        } disabled:cursor-default`}
      >
        {label}
      </button>
    )
  }
  return (
    <div
      className={`flex min-w-0 overflow-clip border border-[#D5D9E0] ${large ? 'rounded-[6px]' : 'rounded-[5px]'}`}
    >
      {option('follow', 'Follow agent')}
      {option('control', 'Control')}
    </div>
  )
}

function Notice({
  live,
  large,
}: {
  live: LiveScreenController
  large: boolean
}) {
  const notice = screenNotice(live.state)
  if (!notice) return null
  return (
    <div
      role="status"
      className={`absolute inset-0 flex flex-col items-center justify-center gap-1 bg-[#F8F9FB] px-4 text-center ${large ? 'text-[13px] leading-5' : 'text-[11px] leading-[14px]'}`}
    >
      <p className="font-semibold text-[#0D1730]">{notice.headline}</p>
      <p className="max-w-[60ch] text-[#4A5365]">{notice.detail}</p>
      {notice.next && <p className="text-[#6B7385]">{notice.next}</p>}
    </div>
  )
}

function ActionError({ live }: { live: LiveScreenController }) {
  if (!live.state.actionError) return null
  return (
    <p role="alert" className="text-[#B42318] text-[11px] leading-[14px]">
      {live.state.actionError}
    </p>
  )
}

function TabChip({ tab }: { tab: LiveTab }) {
  return (
    <span
      className="flex min-w-0 items-center gap-[5px] rounded-[6px] border border-[#D5D9E0] px-[9px] py-1"
      title={tab.url}
    >
      <span className="truncate text-[#0D1730] text-[12px] leading-4">
        {tab.title || tab.url}
      </span>
      <span
        className={`shrink-0 text-[#6B7385] text-[11px] leading-[14px] ${MONO}`}
      >
        tab {tab.tabId}
      </span>
    </span>
  )
}

function PopOutIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 11 11" aria-hidden="true">
      <path
        d="M6.5 1.5h3v3M9.5 1.5L5.5 5.5M8 7v2.5H1.5V3H4"
        fill="none"
        stroke="#0D1730"
        strokeWidth="1.2"
      />
    </svg>
  )
}

function CloseIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
      <path d="M3 3l8 8M11 3l-8 8" stroke="#4A5365" strokeWidth="1.5" />
    </svg>
  )
}
