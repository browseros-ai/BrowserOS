/**
 * The top bar: the viewer's name, the container line, the one MCP URL to give an agent, Clear,
 * which empties the list (the server keeps the previous log once, as `log.prev.jsonl`), and Take
 * over, which hands the agent's tab to you (PR C2's `TakeOverButton`, on the screen's controller).
 * The container line is read-only (owner, 2026-11-11: the command that starts the viewer is the
 * control): the image tag and whether it is up when the viewer started it; otherwise that the
 * image is managed outside the viewer, and where MCP is forwarded.
 */

import { useEffect, useState } from 'react'
import type { ViewerInfo } from '../../shared/api'
import { TakeOverButton } from '../live-screen/LiveScreen'
import type { LiveScreenController } from '../live-screen/use-live-screen'

export function TopBar({
  info,
  onClear,
  live,
}: {
  info: ViewerInfo | null
  onClear: () => void
  live: LiveScreenController
}) {
  return (
    <header className="flex h-14 shrink-0 items-center gap-4 border-line border-b bg-white px-5">
      <div className="flex shrink-0 items-center gap-2.5">
        <div className="flex size-[26px] items-center justify-center rounded-[7px] bg-neo">
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
            <rect
              x="1.5"
              y="2.5"
              width="11"
              height="9"
              rx="1.5"
              fill="none"
              stroke="#FFFFFF"
              strokeWidth="1.5"
            />
            <circle cx="5" cy="7" r="1.1" fill="#FFFFFF" />
            <circle cx="9" cy="7" r="1.1" fill="#FFFFFF" />
          </svg>
        </div>
        <h1 className="font-semibold text-[15px] text-black leading-[18px] tracking-[-0.01em]">
          Neo viewer
        </h1>
      </div>
      <ContainerLine info={info} />
      <div className="flex-1" />
      <McpUrl url={info?.mcpUrl ?? null} />
      <div className="flex h-7 shrink-0 items-center gap-2 border-line border-l pl-3">
        <button
          type="button"
          onClick={onClear}
          title="Empty the list. The previous log is kept once, as log.prev.jsonl."
          className="flex h-[30px] items-center rounded-md border border-control px-3 font-semibold text-ink text-xs leading-4 hover:bg-[#F4F6FA]"
        >
          Clear
        </button>
        <TakeOverButton live={live} />
      </div>
    </header>
  )
}

/** The one URL every agent is given, with Copy. */
function McpUrl({ url }: { url: string | null }) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])
  return (
    <div className="flex h-[30px] shrink-0 items-center overflow-clip rounded-md border border-control">
      <code className="px-2.5 font-code text-ink text-xs leading-4">
        {url ?? '…'}
      </code>
      <button
        type="button"
        disabled={url === null}
        onClick={() => {
          if (url)
            void navigator.clipboard.writeText(url).then(() => setCopied(true))
        }}
        className="flex h-[30px] items-center bg-neo px-3 font-semibold text-white text-xs leading-4 hover:bg-[#1A44BD] disabled:opacity-60"
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  )
}

/** Read-only: what the viewer runs, or that it forwards to a Neo it did not start. */
function ContainerLine({ info }: { info: ViewerInfo | null }) {
  const container = info?.container ?? null
  return (
    <div className="flex h-7 min-w-0 items-center gap-3 border-line border-l pl-5">
      {container ? (
        <>
          <span className="flex shrink-0 items-center gap-1.5">
            <span
              className={`size-2 rounded-full ${container.up ? 'bg-ok' : 'bg-bad'}`}
              aria-hidden="true"
            />
            <span className="font-medium text-[13px] text-black leading-4">
              {container.up ? 'Running' : 'Stopped'}
            </span>
          </span>
          <span
            className="truncate font-code text-ink-2 text-xs leading-4"
            title={`Container ${container.name}`}
          >
            {container.image}
          </span>
        </>
      ) : (
        <>
          <span
            className="size-2 shrink-0 rounded-full border border-ink-4"
            aria-hidden="true"
          />
          <span className="shrink-0 font-medium text-[13px] text-black leading-4">
            Image managed outside the viewer
          </span>
          {info && (
            <span
              className="truncate font-code text-ink-3 text-xs leading-4"
              title="Where the viewer forwards MCP: the Neo SDK image's /mcp"
            >
              → {info.upstreamUrl}
            </span>
          )}
        </>
      )}
    </div>
  )
}
