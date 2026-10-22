/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Folder } from 'lucide-react'
import { cn } from '@/lib/utils'
import { DemoChatGptWindow } from './DemoChatGptWindow'
import { DemoClaudeWindow } from './DemoClaudeWindow'
import { DemoNeoWindow } from './DemoNeoWindow'
import { DemoTerminalWindow } from './DemoTerminalWindow'
import { DemoNeoMark } from './demo-parts'
import type { DemoApp, DemoFrame } from './timeline'

/** macOS draws the dock's hover label in the system face, not the app's. */
const MAC_FONT =
  "font-[-apple-system,BlinkMacSystemFont,'SF_Pro_Text','Helvetica_Neue',sans-serif]"

/** Each clickable dock app: its name, icon, and the agent tag neo shows. */
const APPS: Record<
  DemoApp,
  { label: string; icon: string; agent: string; dockX: string }
> = {
  terminal: {
    label: 'Terminal',
    icon: '/onboarding/dock/terminal.webp',
    agent: 'claude',
    dockX: 'left-[420px]',
  },
  claude: {
    label: 'Claude',
    icon: '/onboarding/dock/claude.webp',
    agent: 'claude',
    dockX: 'left-[474px]',
  },
  chatgpt: {
    label: 'ChatGPT',
    icon: '/onboarding/dock/chatgpt.webp',
    agent: 'codex',
    dockX: 'left-[528px]',
  },
}

/**
 * The 960×480 drawn desktop at one frame of the timeline: the playing app's
 * window and neo on the player's plain ground, with the dock. No menu bar or
 * wallpaper (owner). The dock's Terminal, Claude and ChatGPT tiles are real
 * buttons (round 10): clicking one plays that app's flow from the start.
 * Everything else here is decoration and hidden from assistive tech.
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
  const windowProps = {
    frame: frame.agent,
    front: agentFront,
    fading: frame.fading,
  }
  return (
    <>
      <div aria-hidden>
        <DemoNeoWindow
          frame={frame.neo}
          fading={frame.fading}
          agent={APPS[app].agent}
        />
        {app === 'claude' && (
          <DemoClaudeWindow {...windowProps} spinGlyph={spinGlyph} />
        )}
        {app === 'chatgpt' && <DemoChatGptWindow {...windowProps} />}
        {app === 'terminal' && (
          <DemoTerminalWindow {...windowProps} spinGlyph={spinGlyph} />
        )}
        <SwitchCallout app={app} text={frame.callout} />
        <Pointer app={app} pointer={frame.pointer} />
        <AgentPointer at={frame.neo.agentPointer} agent={APPS[app].agent} />
      </div>
      <Dock frame={frame} onPickApp={onPickApp} />
    </>
  )
}

function TipLabel({ label, on }: { label: string; on: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        'pointer-events-none absolute bottom-[calc(100%+10px)] left-1/2 -translate-x-1/2 whitespace-nowrap rounded-[7px] border border-white/20 bg-[rgb(29_31_36/0.88)] px-2.5 py-1 font-medium text-[12px] text-white leading-4 transition-opacity duration-200',
        MAC_FONT,
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
        on ? 'bg-ink-deep dark:bg-white/80' : 'bg-transparent',
      )}
    />
  )
}

const TILE =
  'grid size-11 place-items-center overflow-hidden rounded-[11px] border'

/**
 * Finder, Terminal, Claude, ChatGPT, neo (round 10: Codex runs inside
 * ChatGPT now, so it has no tile of its own). Terminal, Claude and ChatGPT use
 * the apps' real icons, drawn at 52 px in a 44 px slot because their artwork
 * carries macOS's transparent margin. The playing app's tile has a soft ring;
 * the others lift on hover, so the tiles read as clickable.
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
    <div className="absolute top-[404px] left-1/2 z-6 flex h-16 -translate-x-1/2 items-start gap-2.5 rounded-[20px] border border-white/95 bg-card/60 px-[11px] pt-2 shadow-[0_24px_44px_-18px_rgb(1_18_63/0.38),0_2px_8px_rgb(1_18_63/0.08)] backdrop-blur-[10px] dark:border-white/10">
      <div aria-hidden className="flex flex-col items-center gap-[5px]">
        <span className={cn(TILE, 'border-border-2 bg-card text-accent-2')}>
          <Folder
            aria-hidden
            className="size-6 fill-accent-tint"
            strokeWidth={1.8}
          />
        </span>
        <RunningDot on={false} />
      </div>
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
                'block cursor-pointer rounded-[11px] outline-none transition-[transform,box-shadow] duration-200 focus-visible:ring-4 focus-visible:ring-accent/40',
                selected
                  ? 'ring-2 ring-accent/45 ring-offset-2 ring-offset-card/0'
                  : 'hover:-translate-y-1',
                selected && frame.dock.bounce && 'animate-demo-bounce',
              )}
            >
              <img
                src={APPS[app].icon}
                alt=""
                draggable={false}
                className="-m-1 size-[52px] max-w-none select-none"
              />
            </button>
            <RunningDot on={selected && frame.dock.running} />
            <TipLabel label={APPS[app].label} on={selected && frame.dock.tip} />
          </div>
        )
      })}
      <div aria-hidden className="flex flex-col items-center gap-[5px]">
        <DemoNeoMark className="size-11 rounded-[11px] shadow-[0_10px_18px_-8px_rgb(2_84_236/0.6)]" />
        <RunningDot on />
      </div>
    </div>
  )
}

const ARROW_PATH = 'M1.5 1.5v19.2l4.6-4.4 3 7.1 3.2-1.4-3-6.9h6.3z'

/** Where the toggle's second half (Code, Work) and its callout sit per app. */
const TOGGLE_SPOTS: Record<DemoApp, { pointer: string; callout: string }> = {
  claude: {
    pointer: 'top-[58px] left-[112px]',
    callout: 'top-[49px] left-[160px]',
  },
  chatgpt: {
    pointer: 'top-[31px] left-[333px]',
    callout: 'top-[22px] left-[380px]',
  },
  terminal: {
    pointer: 'top-[428px] left-[420px]',
    callout: 'top-[22px] left-[380px]',
  },
}

/** Names the step people miss: Code in Claude, Work in ChatGPT. */
function SwitchCallout({ app, text }: { app: DemoApp; text: string | null }) {
  return (
    <span
      className={cn(
        'pointer-events-none absolute z-10 flex items-center whitespace-nowrap rounded-[7px] bg-[rgb(29_31_36/0.92)] px-2.5 py-1 font-medium text-[12px] text-white leading-4 shadow-[0_8px_18px_-8px_rgb(0_0_0/0.5)] transition-[opacity,transform] duration-300 ease-out',
        MAC_FONT,
        TOGGLE_SPOTS[app].callout,
        text ? 'opacity-100' : '-translate-x-1 opacity-0',
      )}
    >
      {/* A small tail pointing back at the toggle. */}
      <i className="absolute top-1/2 -left-1 size-2 -translate-y-1/2 rotate-45 bg-[rgb(29_31_36/0.92)]" />
      {text ?? ''}
    </span>
  )
}

/** Your pointer: crosses the desktop to the app's dock tile and opens it,
 * then (Claude, ChatGPT) moves to the toggle and clicks. */
function Pointer({
  app,
  pointer,
}: {
  app: DemoApp
  pointer: DemoFrame['pointer']
}) {
  const spot =
    pointer.at === 'toggle'
      ? TOGGLE_SPOTS[app].pointer
      : pointer.at === 'dock'
        ? cn('top-[428px]', APPS[app].dockX)
        : 'top-[260px] left-[640px]'
  return (
    <svg
      aria-hidden
      viewBox="0 0 18 26"
      className={cn(
        'absolute z-9 h-[26px] w-[18px] transition-[left,top,opacity,transform] duration-[1100ms,1100ms,300ms,120ms] ease-[cubic-bezier(.3,.1,.2,1)]',
        spot,
        pointer.on ? 'opacity-100' : 'opacity-0',
        pointer.pressed && 'scale-[0.88]',
      )}
    >
      <path
        d={ARROW_PATH}
        className="fill-ink-deep stroke-white"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
    </svg>
  )
}

const AGENT_SPOTS = {
  off: 'top-[194px] left-[760px]',
  to: 'top-[130px] left-[738px]',
  when: 'top-[130px] left-[872px]',
  answer: 'top-[274px] left-[784px]',
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
        'absolute z-9 flex flex-col items-start transition-[left,top,opacity] duration-500 ease-out',
        AGENT_SPOTS[at],
        at === 'off' ? 'opacity-0' : 'opacity-100',
      )}
    >
      <svg aria-hidden width="15" height="21" viewBox="0 0 18 26">
        <path
          d={ARROW_PATH}
          className="fill-accent stroke-white"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
      <span className="-mt-1 ml-[11px] flex h-[19px] items-center gap-[5px] rounded-[10px] bg-ink-deep px-[7px] font-mono font-semibold text-[11px] text-white">
        <i className="size-1.5 rounded-full bg-chart-3" />
        {agent}
      </span>
    </div>
  )
}
