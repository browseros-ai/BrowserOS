/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'
import { DemoChatGptWindow } from './DemoChatGptWindow'
import { DemoClaudeWindow } from './DemoClaudeWindow'
import { DemoNeoWindow } from './DemoNeoWindow'
import { DemoTerminalWindow } from './DemoTerminalWindow'
import { DemoNeoMark, PointerArrow } from './demo-parts'
import type { DemoApp, DemoFrame } from './timeline'

/** Each clickable dock app: its name, icon, the agent tag neo shows, and
 * where your pointer rests on its tile (stage px). */
const APPS: Record<
  DemoApp,
  { label: string; icon: string; agent: string; pointer: string }
> = {
  terminal: {
    label: 'Terminal',
    icon: '/onboarding/dock/terminal.webp',
    agent: 'claude',
    pointer: 'top-[456px] left-[411px]',
  },
  claude: {
    label: 'Claude',
    icon: '/onboarding/dock/claude.webp',
    agent: 'claude',
    pointer: 'top-[456px] left-[465px]',
  },
  chatgpt: {
    label: 'ChatGPT',
    icon: '/onboarding/dock/chatgpt.webp',
    agent: 'codex',
    pointer: 'top-[456px] left-[519px]',
  },
}

/**
 * The 976×520 drawn desktop at one frame of the timeline, on Paper's one
 * grid (owner, "same uniform padding everywhere"): S = 20 px from the stage
 * edges, between the two windows and around the dock.
 *
 *   0–28     slim menu bar
 *   48–414   the agent's window (20–424, 44 %) | neo (444–956, 56 %)
 *   434–500  the dock, centred, S above the bottom edge
 *
 * The windows sit side by side with the same top and bottom in every flow and
 * beat, so they never cover each other. The dock's Terminal, Claude and
 * ChatGPT tiles are real buttons (round 10): clicking one plays that app's
 * flow from the start. Everything else is decoration, hidden from assistive
 * tech.
 */
export function DemoDesktop({
  frame,
  spinGlyph,
  onPickApp,
}: {
  frame: DemoFrame
  spinGlyph: string
  onPickApp: (app: DemoApp) => void
}) {
  const app = frame.agent.app
  const agentFront = frame.front === 'agent'
  return (
    <>
      <div aria-hidden>
        <div className="absolute top-0 left-0 h-[680px] w-[1200px] bg-[url(/onboarding/demo-wallpaper.webp)] bg-size-[1200px_680px] bg-no-repeat" />
        <MenuBar name={agentFront ? APPS[app].label : null} />
        <div
          className={cn(
            'transition-opacity duration-500 ease-out',
            frame.fading && 'opacity-0',
          )}
        >
          <div
            className={cn(
              'absolute top-12 left-5 h-[366px] w-[404px] transition-[opacity,transform] duration-200 ease-out',
              !frame.agent.open && 'translate-y-2 opacity-0',
            )}
          >
            {app === 'claude' && (
              <DemoClaudeWindow frame={frame.agent} front={agentFront} />
            )}
            {app === 'chatgpt' && (
              <DemoChatGptWindow frame={frame.agent} front={agentFront} />
            )}
            {app === 'terminal' && (
              <DemoTerminalWindow frame={frame.agent} spinGlyph={spinGlyph} />
            )}
          </div>
          <div className="absolute top-12 left-[444px] h-[366px] w-[512px]">
            <DemoNeoWindow frame={frame.neo} agent={APPS[app].agent} />
          </div>
        </div>
        <SwitchCallout app={app} text={frame.callout} />
        <Pointer app={app} pointer={frame.pointer} />
        <AgentPointer at={frame.neo.agentPointer} agent={APPS[app].agent} />
      </div>
      <Dock frame={frame} onPickApp={onPickApp} />
    </>
  )
}

/**
 * The slim menu bar (round 11): the frontmost app's name in bold after a
 * glyph (neo's dog for neo, a neutral square otherwise), then wifi, battery
 * and the clock. No menus and no "1 agent working" (owner).
 */
function MenuBar({ name }: { name: string | null }) {
  return (
    <div className="absolute top-0 left-0 z-7 flex h-7 w-[976px] items-center gap-[9px] border-[#CBD5E1]/60 border-b bg-white/74 px-4 text-[#01123F]">
      {name ? (
        <i className="size-3 flex-none rounded-[3.5px] bg-[#717783]" />
      ) : (
        <DemoNeoMark className="size-[15px] rounded-[4px]" />
      )}
      <b className="font-bold text-[13px] leading-4">
        {name ?? 'BrowserOS neo'}
      </b>
      <span className="flex-1" />
      <span className="flex items-center gap-4 font-medium text-[13px] leading-4">
        <svg aria-hidden width="16" height="12" viewBox="0 0 16 12">
          <path
            d="M1.5 4.2a9.5 9.5 0 0 1 13 0M3.9 6.7a6 6 0 0 1 8.2 0M6.3 9.1a2.6 2.6 0 0 1 3.4 0"
            fill="none"
            stroke="#01123F"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
        </svg>
        <svg aria-hidden width="24" height="12" viewBox="0 0 24 12">
          <rect
            x="0.75"
            y="0.75"
            width="19.5"
            height="10.5"
            rx="3"
            fill="none"
            stroke="rgb(1 18 63 / 0.55)"
            strokeWidth="1.2"
          />
          <rect x="2.5" y="2.5" width="14" height="7" rx="1.6" fill="#01123F" />
          <path
            d="M22 4.2v3.6"
            fill="none"
            stroke="rgb(1 18 63 / 0.55)"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
        Thu Oct 22 9:41 AM
      </span>
    </div>
  )
}

function TipLabel({ label, on }: { label: string; on: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        'pointer-events-none absolute bottom-[calc(100%+7px)] left-1/2 z-8 flex h-6 -translate-x-1/2 items-center whitespace-nowrap rounded-[7px] bg-[rgb(27_34_54/0.95)] px-2.5 font-medium text-[12px] text-white leading-4 shadow-[0_0_0_1px_rgb(255_255_255/0.15),0_4px_12px_rgb(1_18_63/0.25)] transition-opacity duration-200',
        on ? 'opacity-100' : 'opacity-0',
      )}
    >
      {label}
    </span>
  )
}

function RunningDot({ on }: { on: boolean }) {
  return (
    <i
      aria-hidden
      className={cn(
        'size-1 rounded-full',
        on ? 'bg-[#01123F]' : 'bg-transparent',
      )}
    />
  )
}

/**
 * Terminal · Claude · ChatGPT · BrowserOS neo (no Finder, no separator, and
 * no Codex: it runs inside ChatGPT's Work mode). Real app icons at 44 px. The
 * playing app's tile has a soft blue ring; the others lift on hover, so the
 * tiles read as clickable.
 */
function Dock({
  frame,
  onPickApp,
}: {
  frame: DemoFrame
  onPickApp: (app: DemoApp) => void
}) {
  const playing = frame.agent.app
  return (
    <div className="absolute top-[434px] left-1/2 z-6 flex h-[66px] -translate-x-1/2 items-start gap-2.5 rounded-[22px] border border-white/95 bg-white/66 px-3 pt-2 shadow-[0_24px_44px_-18px_rgb(1_18_63/0.38),0_2px_8px_rgb(1_18_63/0.08)]">
      {(['terminal', 'claude', 'chatgpt'] as const).map((app) => {
        const selected = app === playing
        return (
          <div
            key={app}
            className="relative flex flex-col items-center gap-[5px]"
          >
            <button
              type="button"
              aria-label={`Show it in ${APPS[app].label}`}
              aria-pressed={selected}
              onClick={() => onPickApp(app)}
              className={cn(
                'block size-11 cursor-pointer rounded-[11px] outline-none transition-[transform,box-shadow] duration-200 focus-visible:shadow-[0_0_0_2px_#fff,0_0_0_5px_rgb(2_84_236/0.7)]',
                selected
                  ? 'shadow-[0_0_0_2px_#fff,0_0_0_4.5px_rgb(2_84_236/0.5)]'
                  : 'hover:-translate-y-1',
                selected && frame.dock.bounce && 'animate-demo-bounce',
              )}
            >
              <img
                src={APPS[app].icon}
                alt=""
                draggable={false}
                className="size-11 select-none"
              />
            </button>
            <RunningDot on={selected && frame.dock.running} />
            <TipLabel label={APPS[app].label} on={selected && frame.dock.tip} />
          </div>
        )
      })}
      <div aria-hidden className="flex flex-col items-center gap-[5px]">
        <DemoNeoMark className="size-11 rounded-xl" />
        <RunningDot on />
      </div>
    </div>
  )
}

/**
 * Where each callout sits, inside the agent's window: beside Claude's sidebar
 * toggle and Terminal's command, and under ChatGPT's centred toggle, pointing
 * up at Work. Your pointer's spot on each toggle's second half sits with it.
 */
const SWITCH_SPOTS: Record<
  DemoApp,
  { callout: string; up: boolean; pointer: string | null }
> = {
  claude: {
    callout: 'top-[84px] left-[164px]',
    up: false,
    pointer: 'top-[100px] left-[126px]',
  },
  chatgpt: {
    callout: 'top-[85px] left-[323px] -translate-x-1/2',
    up: true,
    pointer: 'top-[68px] left-[326px]',
  },
  terminal: { callout: 'top-[88px] left-[120px]', up: false, pointer: null },
}

const CALLOUT_FILL = 'rgb(27 34 54 / 0.95)'

/** Names the step people miss: Code in Claude, Work in ChatGPT, typing
 * `claude` in Terminal. */
function SwitchCallout({ app, text }: { app: DemoApp; text: string | null }) {
  const { callout, up } = SWITCH_SPOTS[app]
  return (
    <span
      className={cn(
        'pointer-events-none absolute z-10 flex items-center transition-[opacity,translate] duration-300 ease-out',
        up && 'flex-col',
        callout,
        text
          ? 'opacity-100'
          : cn('opacity-0', up ? '-translate-y-1' : '-translate-x-1'),
      )}
    >
      {up ? (
        <svg aria-hidden width="12" height="6" viewBox="0 0 12 6">
          <path d="M0 6L6 0l6 6z" fill={CALLOUT_FILL} />
        </svg>
      ) : (
        <svg aria-hidden width="6" height="12" viewBox="0 0 6 12">
          <path d="M6 0L0 6l6 6z" fill={CALLOUT_FILL} />
        </svg>
      )}
      <span className="flex h-7 items-center whitespace-nowrap rounded-lg bg-[rgb(27_34_54/0.95)] px-[11px] font-semibold text-[13px] text-white leading-4 shadow-[0_0_0_1px_rgb(255_255_255/0.15),0_6px_16px_rgb(1_18_63/0.25)]">
        {text ?? ''}
      </span>
    </span>
  )
}

/** Your pointer: crosses the desktop to the app's dock tile and opens it,
 * then (Claude, ChatGPT) clicks the toggle and comes back to rest on it. */
function Pointer({
  app,
  pointer,
}: {
  app: DemoApp
  pointer: DemoFrame['pointer']
}) {
  const spot =
    pointer.at === 'toggle'
      ? SWITCH_SPOTS[app].pointer
      : pointer.at === 'dock'
        ? APPS[app].pointer
        : 'top-[265px] left-[650px]'
  return (
    <PointerArrow
      className={cn(
        'absolute z-9 transition-[left,top,opacity,scale] duration-[1100ms,1100ms,300ms,120ms] ease-[cubic-bezier(.3,.1,.2,1)]',
        spot,
        pointer.on ? 'opacity-100' : 'opacity-0',
        pointer.pressed && 'scale-88',
      )}
    />
  )
}

const AGENT_SPOTS = {
  off: 'top-[250px] left-[792px]',
  to: 'top-[170px] left-[704px]',
  when: 'top-[170px] left-[860px]',
  answer: 'top-[330px] left-[792px]',
} as const

/** The agent's pointer, tagged with the agent, doing the work inside neo. */
function AgentPointer({
  at,
  agent,
}: {
  at: DemoFrame['neo']['agentPointer']
  agent: string
}) {
  return (
    <div
      aria-hidden
      className={cn(
        'absolute z-9 flex flex-col items-start gap-0.5 transition-[left,top,opacity] duration-[900ms,900ms,300ms] ease-[cubic-bezier(.3,.1,.2,1)]',
        AGENT_SPOTS[at],
        at === 'off' ? 'opacity-0' : 'opacity-100',
      )}
    >
      <PointerArrow />
      <span className="flex h-5 items-center gap-[5px] rounded-[10px] bg-[#01123F] px-2 font-medium font-mono text-[11px] text-white leading-3 shadow-[0_4px_10px_rgb(1_18_63/0.25)]">
        <i className="size-1.5 rounded-full bg-[#F5B957]" />
        {agent}
      </span>
    </div>
  )
}
