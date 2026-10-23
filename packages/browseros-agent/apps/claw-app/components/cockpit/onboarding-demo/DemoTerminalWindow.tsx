/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'
import { TOOL_CALLS, TrafficLights, Transcript } from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

/** Terminal's block cursor: where the next keystroke lands. */
function BlockCursor({ className }: { className?: string }) {
  return (
    <i
      aria-hidden
      className={cn(
        'inline-block h-4 w-2 flex-none animate-demo-blink bg-[#EAF1FF]/35 align-[-3px]',
        className,
      )}
    />
  )
}

/**
 * Claude Code in a terminal, in round 4's look: type `claude` and press
 * return, Claude Code starts and shows it is connected to neo, then the ask
 * goes in at its `>` prompt (round 10). The tool calls fit on one line each,
 * and each number lights its badge in neo. Paper draws this window only at
 * the 1200 px beat size; this is that design fitted to the cockpit's grid.
 */
export function DemoTerminalWindow({
  frame,
  spinGlyph,
}: {
  frame: DemoFrame['agent']
  spinGlyph: string
}) {
  const started = frame.switched
  return (
    <div className="flex size-full flex-col overflow-hidden rounded-xl bg-[#01123F] font-mono text-white shadow-[0_0_0_1px_rgb(1_18_63/0.4),0_34px_60px_-28px_rgb(1_18_63/0.6),0_10px_24px_-12px_rgb(1_18_63/0.35)]">
      <div className="relative flex h-8 flex-none items-center border-[#EAF1FF]/8 border-b px-3">
        <TrafficLights size={11} />
        <span className="absolute inset-x-0 top-[9px] text-center text-[#8C97B3] text-[11.5px] leading-[14px]">
          {started ? 'claude — ~' : 'zsh — ~'}
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-[9px] p-3">
        <Transcript gap={9}>
          <div className="flex items-center gap-2 text-[13px] leading-5">
            <span className="text-[#7E8FB8]">~ %</span>
            <span className="font-medium">
              {frame.command}
              {!frame.commandRun && <BlockCursor className="ml-px" />}
            </span>
          </div>
          {started && (
            <div className="flex flex-none animate-demo-rise items-center gap-3 rounded-lg border border-[#F5B957]/70 px-3 py-2">
              <svg aria-hidden width="18" height="18" viewBox="0 0 18 18">
                <path
                  d="M9 2v14M2.9 5.5l12.2 7M2.9 12.5l12.2-7"
                  fill="none"
                  stroke="#F5B957"
                  strokeWidth="2"
                  strokeLinecap="round"
                />
              </svg>
              <div>
                <b className="block font-semibold text-[13px] leading-4">
                  Claude Code
                </b>
                <span className="mt-0.5 block text-[#8C97B3] text-[11.5px] leading-[14px]">
                  browseros-neo ✓ connected
                </span>
              </div>
            </div>
          )}
          {frame.sent && (
            <div className="flex animate-demo-rise gap-[9px] text-[13px] leading-5">
              <span className="text-[#8C97B3]">&gt;</span>
              <span>{DEMO_PROMPT}</span>
            </div>
          )}
          <div className="flex flex-col gap-1">
            {TOOL_CALLS.slice(0, frame.tools).map((tool, index) => {
              const lit = frame.litTool === index
              return (
                <div
                  key={tool.name}
                  className={cn(
                    'flex h-[30px] flex-none animate-demo-rise items-center gap-1.5 whitespace-nowrap rounded-md border px-[7px] text-[11.5px] leading-4 transition-[background-color,border-color] duration-500 ease-out',
                    lit
                      ? 'border-[#0254EC] bg-[#0254EC]/30'
                      : 'border-transparent',
                  )}
                >
                  <span
                    className={cn(
                      'grid size-[18px] flex-none place-items-center rounded-full font-bold text-[11px] leading-3 transition-[background-color,box-shadow] duration-500',
                      lit
                        ? 'bg-[#0254EC] shadow-[0_0_0_2px_rgb(211_226_255/0.55)]'
                        : 'bg-[#0254EC]/55',
                    )}
                  >
                    {index + 1}
                  </span>
                  <span className="text-[#EAF1FF]">{tool.name}</span>
                  <span className="text-[#8C97B3] text-[11px]">{tool.arg}</span>
                </div>
              )
            })}
          </div>
          {frame.working && (
            <div className="flex animate-demo-rise items-center gap-2 px-2 pt-0.5 text-[#8C97B3] text-[12px] leading-4">
              <b className="font-semibold text-[#F5B957] text-[12.5px]">
                <span className="inline-block w-3 text-center">
                  {spinGlyph}
                </span>{' '}
                Browsing…
              </b>
              <span>({frame.seconds}s · esc to interrupt)</span>
            </div>
          )}
          {frame.answered && (
            <div className="flex animate-demo-rise items-start gap-2 px-2 pt-0.5 font-sans font-semibold text-[13px] leading-[18px]">
              <span className="mt-px grid size-4 flex-none place-items-center rounded-full bg-white/14">
                <svg aria-hidden width="9" height="9" viewBox="0 0 16 16">
                  <path
                    d="M3.5 8.5l3 3 6-7"
                    fill="none"
                    stroke="#fff"
                    strokeWidth="2.4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </span>
              Cheapest is $142, nonstop, Fri 7:05 am.
            </div>
          )}
        </Transcript>
        <div
          className={cn(
            'flex min-h-9 flex-none items-start gap-[9px] rounded-[7px] border border-[#EAF1FF]/14 px-3 py-[9px] text-[13px] leading-4',
            !started && 'invisible',
          )}
        >
          <span className="text-[#8C97B3]">&gt;</span>
          <span className="min-w-0 flex-1">
            {frame.composer}
            <BlockCursor className={cn(frame.composer && 'ml-px')} />
          </span>
        </div>
      </div>
    </div>
  )
}
