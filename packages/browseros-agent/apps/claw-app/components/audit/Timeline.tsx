import {
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Copy,
  Image as ImageIcon,
} from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { CodeBlock } from '@/components/ai-elements/code-block'
import { AspectRatio } from '@/components/ui/aspect-ratio'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  type ToolDispatchRow,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { parseResultMeta } from '@/screens/audit/audit.helpers'

interface TimelineProps {
  dispatches: ToolDispatchRow[]
  parentToolNames?: ReadonlyMap<string, string>
  startedAt: number
  endEvent: {
    createdAt: number
    kind: 'closed' | 'errored' | 'cancelled'
    reason: string | null
  } | null
  /**
   * Whether to render the session-end row below the dispatch list.
   * Default true keeps existing consumers unchanged. Per-tab views
   * (see `TabView.tsx`) pass false because the session end is not
   * scoped to any one tab; it lives on the Session tab only.
   */
  showSessionEnd?: boolean
  onScreenshotClick: (screenshotId: number) => void
}

const NOTABLE_TOOLS = new Set([
  'act',
  'evaluate',
  'run',
  'playwright',
  'download',
])

/** Links script steps by wire dispatch key; numeric IDs only identify UI rows. */
interface DispatchNode {
  dispatch: ToolDispatchRow
  children: DispatchNode[]
  code: string | null
}

function groupDispatches(dispatches: ToolDispatchRow[]): DispatchNode[] {
  const nodes = dispatches.map(
    (dispatch): DispatchNode => ({
      dispatch,
      children: [],
      code: scriptCode(dispatch.argsJson),
    }),
  )
  const byKey = new Map(
    nodes.flatMap((node) =>
      node.dispatch.dispatchKey
        ? [[node.dispatch.dispatchKey, node] as const]
        : [],
    ),
  )
  const roots: DispatchNode[] = []
  for (const node of nodes) {
    const parent = node.dispatch.parentDispatchId
      ? byKey.get(node.dispatch.parentDispatchId)
      : undefined
    // Children can arrive before their parent and per-tab lists omit page-less
    // parents. Keep those steps visible at the top level until a parent exists.
    if (parent && parent !== node) parent.children.push(node)
    else roots.push(node)
  }
  for (const node of nodes) {
    node.children.sort(
      (a, b) =>
        a.dispatch.createdAt - b.dispatch.createdAt ||
        a.dispatch.dispatchId - b.dispatch.dispatchId,
    )
  }
  return roots
}

function isNotable(node: DispatchNode): boolean {
  return (
    NOTABLE_TOOLS.has(node.dispatch.toolName) ||
    node.children.length > 0 ||
    node.code !== null
  )
}

function scriptCode(argsJson: string | undefined): string | null {
  if (!argsJson) return null
  try {
    const args: unknown = JSON.parse(argsJson)
    return args !== null &&
      typeof args === 'object' &&
      'code' in args &&
      typeof args.code === 'string'
      ? args.code
      : null
  } catch {
    return null
  }
}

export function Timeline({
  dispatches,
  parentToolNames,
  startedAt,
  endEvent,
  showSessionEnd = true,
  onScreenshotClick,
}: TimelineProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const roots = useMemo(() => groupDispatches(dispatches), [dispatches])
  // Polling may deliver a script parent after its steps. Defaults apply to new
  // rows too, while explicit user toggles survive regrouping and later polls.
  const [expansionOverrides, setExpansionOverrides] = useState(
    new Map<number, boolean>(),
  )
  const expanded = new Set<number>()
  const collectExpanded = (node: DispatchNode): void => {
    if (expansionOverrides.get(node.dispatch.dispatchId) ?? isNotable(node)) {
      expanded.add(node.dispatch.dispatchId)
    }
    node.children.forEach(collectExpanded)
  }
  roots.forEach(collectExpanded)
  const toggle = (id: number): void =>
    setExpansionOverrides((prev) => new Map(prev).set(id, !expanded.has(id)))
  const expandAll = (): void =>
    setExpansionOverrides(new Map(dispatches.map((d) => [d.dispatchId, true])))
  const collapseAll = (): void =>
    setExpansionOverrides(new Map(dispatches.map((d) => [d.dispatchId, false])))
  const allExpanded =
    dispatches.length > 0 && dispatches.every((d) => expanded.has(d.dispatchId))
  const noneExpanded = expanded.size === 0

  const renderRow = (node: DispatchNode, nested = false): ReactNode => (
    <TimelineRow
      key={node.dispatch.dispatchId}
      dispatch={node.dispatch}
      offsetMs={Math.max(0, node.dispatch.createdAt - startedAt)}
      expanded={expanded.has(node.dispatch.dispatchId)}
      notable={isNotable(node)}
      code={node.code}
      parentToolName={
        !nested && node.dispatch.parentDispatchId
          ? parentToolNames?.get(node.dispatch.parentDispatchId)
          : undefined
      }
      childCount={node.children.length}
      hasChildError={node.children.some(
        (child) => parseResultMeta(child.dispatch.resultMeta)?.isError === true,
      )}
      screenshotBaseUrl={screenshotBaseUrl}
      onToggle={() => toggle(node.dispatch.dispatchId)}
      onScreenshotClick={onScreenshotClick}
    >
      {node.children.length > 0 && (
        <ol className="mt-3 ml-4 space-y-1.5 border-border-2 border-l pl-3">
          {node.children.map((child) => renderRow(child, true))}
        </ol>
      )}
    </TimelineRow>
  )

  return (
    <section className="rounded-2xl border border-border-2 bg-card p-4">
      <header className="flex items-center justify-between gap-3 pb-3">
        <h2 className="font-semibold text-ink">Timeline</h2>
        <div className="flex items-center gap-1.5">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={expandAll}
            disabled={allExpanded || dispatches.length === 0}
            className="h-7 gap-1 px-2 text-[11.5px] text-ink-3 hover:text-ink"
            data-testid="timeline-expand-all"
          >
            <ChevronsUpDown className="size-3.5" />
            Expand all
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={collapseAll}
            disabled={noneExpanded}
            className="h-7 gap-1 px-2 text-[11.5px] text-ink-3 hover:text-ink"
            data-testid="timeline-collapse-all"
          >
            <ChevronsDownUp className="size-3.5" />
            Collapse all
          </Button>
          <span className="pl-2 text-[12.5px] text-ink-3">
            {dispatches.length} event{dispatches.length === 1 ? '' : 's'}
          </span>
        </div>
      </header>
      <ol className="space-y-1.5">
        {roots.map((node) => renderRow(node))}
        {showSessionEnd && (
          <SessionEndRow startedAt={startedAt} endEvent={endEvent} />
        )}
      </ol>
    </section>
  )
}

interface TimelineRowProps {
  dispatch: ToolDispatchRow
  offsetMs: number
  expanded: boolean
  notable: boolean
  code: string | null
  parentToolName?: string
  childCount: number
  hasChildError: boolean
  children: ReactNode
  screenshotBaseUrl: string | null
  onToggle: () => void
  onScreenshotClick: (screenshotId: number) => void
}

function TimelineRow({
  dispatch,
  offsetMs,
  expanded,
  notable,
  code,
  parentToolName,
  childCount,
  hasChildError,
  children,
  screenshotBaseUrl,
  onToggle,
  onScreenshotClick,
}: TimelineRowProps) {
  const meta = parseResultMeta(dispatch.resultMeta)
  const isError = meta?.isError ?? false
  const screenshotId = dispatch.screenshotId
  const isScreenshot = screenshotId !== undefined
  return (
    <li
      className={cn(
        'rounded-lg border border-transparent px-2 py-1.5',
        notable && 'border-primary/30 bg-primary/5',
        isError && 'border-red-500/30 bg-red-500/5',
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        className={cn(
          'grid w-full grid-cols-[auto_5rem_minmax(0,1fr)_auto] items-center gap-3 rounded-md px-1 py-1 text-left transition-colors',
          // Hover-tint only the header, never the body. Otherwise the
          // tint matches the args / result codeblock backgrounds and the
          // codeblocks visually disappear into the row.
          'hover:bg-card-tint',
        )}
      >
        {expanded ? (
          <ChevronDown className="size-3.5 text-ink-3" />
        ) : (
          <ChevronRight className="size-3.5 text-ink-3" />
        )}
        <span className="font-mono text-[11.5px] text-ink-3">
          T+{formatOffset(offsetMs)}
        </span>
        <div className="min-w-0 space-y-0.5">
          {parentToolName && (
            <span className="block text-[11px] text-ink-3">
              in {parentToolName} script
            </span>
          )}
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="font-mono font-semibold text-[12.5px] text-ink [overflow-wrap:anywhere]">
              {dispatch.toolName}
            </span>
            {childCount > 0 && (
              <span className="shrink-0 rounded bg-card-tint px-1.5 text-[11px] text-ink-3">
                {childCount} {childCount === 1 ? 'step' : 'steps'}
              </span>
            )}
            {(isError || hasChildError) && (
              <span className="shrink-0 rounded bg-red-500/10 px-1.5 text-[11px] text-red-500">
                {hasChildError ? 'Step failed' : 'Failed'}
              </span>
            )}
          </div>
          <span className="line-clamp-2 text-[12.5px] text-ink-3 [overflow-wrap:anywhere]">
            {argsSummary(code ?? dispatch.argsJson)}
          </span>
        </div>
        <span className="font-mono text-[11.5px] text-ink-3">
          {dispatch.durationMs ?? 0}ms
        </span>
      </button>
      {expanded && (
        <div className="mt-2 space-y-2 border-border-2 border-t px-1 pt-2">
          {code !== null ? (
            <ScriptCode code={code} />
          ) : (
            dispatch.argsJson && (
              <Block label="args" copyText={dispatch.argsJson}>
                <pre className="overflow-x-auto whitespace-pre-wrap break-all text-[11.5px]">
                  {dispatch.argsJson}
                </pre>
              </Block>
            )
          )}
          {dispatch.resultMeta && (
            <Block label="result" copyText={dispatch.resultMeta}>
              <pre className="overflow-x-auto whitespace-pre-wrap break-all text-[11.5px]">
                {dispatch.resultMeta}
              </pre>
            </Block>
          )}
          {isScreenshot && screenshotBaseUrl !== null && (
            <Block label="screenshot">
              <button
                type="button"
                onClick={() => onScreenshotClick(screenshotId)}
                className="block w-64 max-w-full overflow-hidden rounded-md border border-border-2"
              >
                <AspectRatio ratio={16 / 10}>
                  <img
                    src={taskScreenshotUrl(
                      dispatch.sessionId,
                      screenshotId,
                      screenshotBaseUrl,
                    )}
                    alt={`Screenshot at T+${formatOffset(offsetMs)}`}
                    className="h-full w-full object-contain"
                    loading="lazy"
                  />
                </AspectRatio>
              </button>
            </Block>
          )}
          {isScreenshot && screenshotBaseUrl === null && (
            <Block label="screenshot">
              <div className="w-64 max-w-full overflow-hidden rounded-md border border-border-2">
                <AspectRatio ratio={16 / 10}>
                  <div className="h-full w-full animate-pulse bg-card-tint" />
                </AspectRatio>
              </div>
            </Block>
          )}
          {dispatch.url && (
            <Block label="page" copyText={dispatch.url}>
              <a
                href={dispatch.url}
                target="_blank"
                rel="noreferrer"
                className="text-[12.5px] text-accent [overflow-wrap:anywhere] hover:underline"
              >
                {dispatch.url}
              </a>
            </Block>
          )}
          {!dispatch.argsJson &&
            !dispatch.resultMeta &&
            !isScreenshot &&
            !dispatch.url && (
              <div className="text-[12px] text-ink-3">
                <ImageIcon className="mr-1 inline size-3" />
                No extra detail recorded.
              </div>
            )}
          {children}
        </div>
      )}
    </li>
  )
}

const SCRIPT_PREVIEW_LINES = 40

function ScriptCode({ code }: { code: string }) {
  const [showAll, setShowAll] = useState(false)
  const lines = code.split('\n')
  const truncated = lines.length > SCRIPT_PREVIEW_LINES
  const visibleCode = showAll
    ? code
    : lines.slice(0, SCRIPT_PREVIEW_LINES).join('\n')
  return (
    <Block label="code" copyText={code}>
      <CodeBlock
        code={visibleCode}
        language="javascript"
        // Keep large scripts bounded so their child steps remain reachable.
        // Wrapping applies before and after Shiki replaces raw text with tokens.
        className="max-h-80 overflow-auto border-0 [&_code]:text-[11.5px] [&_pre]:whitespace-pre-wrap [&_pre]:p-0 [&_pre]:[overflow-wrap:anywhere]"
      />
      {truncated && (
        <button
          type="button"
          onClick={() => setShowAll((prev) => !prev)}
          aria-expanded={showAll}
          className="mt-2 text-[11.5px] text-accent hover:underline"
        >
          {showAll ? 'Show less' : `Show all ${lines.length} lines`}
        </button>
      )}
    </Block>
  )
}

function Block({
  label,
  copyText,
  children,
}: {
  label: string
  copyText?: string
  children: React.ReactNode
}) {
  const [copied, setCopied] = useState(false)
  const handleCopy = (): void => {
    if (copyText === undefined) return
    void navigator.clipboard.writeText(copyText).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-2">
        <div className="font-mono font-semibold text-[10.5px] text-ink-3 uppercase tracking-wide">
          {label}
        </div>
        {copyText !== undefined && (
          <button
            type="button"
            onClick={handleCopy}
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[10.5px] text-ink-3 uppercase tracking-wide transition-colors hover:bg-card-tint hover:text-ink"
            aria-label={`Copy ${label}`}
            data-testid={`timeline-block-copy-${label}`}
          >
            {copied ? (
              <Check className="size-3" />
            ) : (
              <Copy className="size-3" />
            )}
            {copied ? 'copied' : 'copy'}
          </button>
        )}
      </div>
      <div className="rounded-md bg-bg-sunken p-2">{children}</div>
    </div>
  )
}

function SessionEndRow({
  startedAt,
  endEvent,
}: {
  startedAt: number
  endEvent: TimelineProps['endEvent']
}) {
  if (!endEvent) {
    return (
      <li className="flex items-center gap-3 px-2 py-1.5 text-[12.5px] text-ink-3">
        <span className="inline-block size-2 animate-pulse rounded-full bg-accent" />
        Still running, no session-close received yet.
      </li>
    )
  }
  const offset = Math.max(0, endEvent.createdAt - startedAt)
  return (
    <li className="flex items-center gap-3 px-2 py-1.5 text-[12.5px] text-ink-3">
      <span className="inline-block size-2 rounded-full bg-ink-3" />
      <span className="font-mono">T+{formatOffset(offset)}</span>
      <span>
        session{' '}
        {endEvent.kind === 'closed'
          ? 'closed'
          : endEvent.kind === 'cancelled'
            ? 'stopped'
            : `errored (${endEvent.reason ?? 'unknown'})`}
      </span>
    </li>
  )
}

function formatOffset(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(2)}s`
  const totalSec = Math.floor(seconds)
  const mins = Math.floor(totalSec / 60)
  const rem = totalSec % 60
  return `${mins}m${rem.toString().padStart(2, '0')}s`
}

function argsSummary(argsJson: string | null | undefined): string {
  if (!argsJson || argsJson === '{}') return ''
  // Let the row's line clamp adapt to available width; cutting at 80 characters
  // hid the useful end of selectors even when there was room to display it.
  return argsJson
}
