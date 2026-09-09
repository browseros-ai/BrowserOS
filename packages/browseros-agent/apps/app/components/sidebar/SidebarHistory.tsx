import { ChevronDown, History, Loader2, Search } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { Link } from 'react-router'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { useActiveConversation } from '@/modules/conversations/active-conversation-context'
import { useServerConversations } from '@/modules/conversations/conversations.hooks'
import {
  conversationTitle,
  HISTORY_PAGE_SIZE,
  historyList,
  historyTimestamp,
} from '@/modules/conversations/history-list'

/** Compact navigation into SQLite history. It never queries the legacy cloud archive. */
export function SidebarHistory({
  expanded,
  onNavigate,
}: {
  expanded: boolean
  onNavigate?: () => void
}) {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')
  const [limit, setLimit] = useState(HISTORY_PAGE_SIZE)
  const [now, setNow] = useState(() => new Date())
  const { id: activeId } = useActiveConversation()
  const contentId = useId()
  const visible = open && expanded
  // Poll only while visible: another tab or the side panel can finish a turn
  // independently of this window's query cache.
  const { data, isPending, isError, refetch } = useServerConversations(
    visible,
    5_000,
  )
  const { groups, hasMore, total } = historyList(data ?? [], search, limit, now)

  useEffect(() => {
    if (!visible) return
    setNow(new Date())
    const interval = setInterval(() => setNow(new Date()), 60_000)
    return () => clearInterval(interval)
  }, [visible])

  const trigger = (
    <button
      type="button"
      aria-label="History"
      aria-expanded={visible}
      aria-controls={contentId}
      onClick={() => setOpen((value) => !value)}
      className={cn(
        'flex h-9 w-full items-center gap-2 overflow-hidden whitespace-nowrap rounded-md px-3 font-medium text-sm transition-colors hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        open && 'bg-[var(--accent-orange)]/10',
      )}
    >
      <History className="size-4 shrink-0" />
      <span
        className={cn(
          'flex-1 text-left transition-opacity duration-200',
          !expanded && 'opacity-0',
        )}
      >
        History
      </span>
      {expanded && (
        <ChevronDown
          className={cn(
            'size-3.5 shrink-0 text-muted-foreground transition-transform',
            !open && '-rotate-90',
          )}
        />
      )}
    </button>
  )

  return (
    <div>
      {/* Keep the trigger mounted while expanding; replacing it on focus can swallow the first click. */}
      {/* Content is absent while expanded; close from the trigger rather than the content's hover region. */}
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>{trigger}</TooltipTrigger>
        {!expanded && <TooltipContent side="right">History</TooltipContent>}
      </Tooltip>
      <div id={contentId} hidden={!visible}>
        <div className="mx-3 my-3 space-y-2">
          <label className="flex h-8 items-center gap-2 rounded-md border bg-background px-2 focus-within:ring-2 focus-within:ring-ring">
            <Search className="size-3.5 shrink-0 text-muted-foreground" />
            <input
              type="search"
              aria-label="Search conversations"
              placeholder="Search conversations"
              value={search}
              onChange={(event) => {
                setSearch(event.target.value)
                setLimit(HISTORY_PAGE_SIZE)
              }}
              className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
            />
          </label>
          <section
            className="styled-scrollbar max-h-[min(28rem,50dvh)] space-y-2 overflow-y-auto"
            aria-label="Recent conversations"
          >
            {isPending && !isError && (
              <p
                role="status"
                className="flex items-center gap-2 px-2 py-3 text-muted-foreground text-xs"
              >
                <Loader2 className="size-3.5 animate-spin" />
                Loading conversations…
              </p>
            )}
            {isError && (
              <div
                role="alert"
                className="px-2 py-2 text-muted-foreground text-xs"
              >
                <p>Couldn’t load history.</p>
                <button
                  type="button"
                  onClick={() => void refetch()}
                  className="mt-1 underline underline-offset-2"
                >
                  Try again
                </button>
              </div>
            )}
            {!isPending && !isError && total === 0 && (
              <p
                role="status"
                className="px-2 py-3 text-muted-foreground text-xs"
              >
                {search.trim()
                  ? 'No conversations found.'
                  : 'Your conversations will appear here.'}
              </p>
            )}
            {groups.map((group) => (
              <section key={group.label} aria-label={group.label}>
                <h3 className="px-2 pt-2 pb-1 font-medium text-muted-foreground text-xs">
                  {group.label}
                </h3>
                <ul className="space-y-1">
                  {group.conversations.map((conversation) => {
                    const title = conversationTitle(
                      conversation.lastUserMessage,
                    )
                    return (
                      <li key={conversation.id}>
                        <Link
                          to={`/home/chat?conversationId=${encodeURIComponent(conversation.id)}`}
                          onClick={onNavigate}
                          aria-current={
                            activeId === conversation.id ? 'page' : undefined
                          }
                          title={title}
                          className={cn(
                            'flex h-10 min-w-0 flex-col justify-center rounded-md px-2 hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                            activeId === conversation.id && 'bg-sidebar-accent',
                          )}
                        >
                          <span className="truncate text-[13px] leading-[19px]">
                            {title}
                          </span>
                          <time
                            dateTime={new Date(
                              conversation.lastMessagedAt,
                            ).toISOString()}
                            className="text-muted-foreground text-xs leading-[18px]"
                          >
                            {historyTimestamp(conversation.lastMessagedAt, now)}
                          </time>
                        </Link>
                      </li>
                    )
                  })}
                </ul>
              </section>
            ))}
            {hasMore && (
              <button
                type="button"
                onClick={() => setLimit((value) => value + HISTORY_PAGE_SIZE)}
                className="w-full rounded-md px-2 py-2 text-left text-muted-foreground text-xs hover:bg-sidebar-accent"
              >
                Show older
              </button>
            )}
          </section>
        </div>
      </div>
    </div>
  )
}
