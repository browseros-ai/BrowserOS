import { Loader2, MessageSquare } from 'lucide-react'
import { type FC, useEffect, useRef } from 'react'
import { Link } from 'react-router'
import { ConversationGroup } from './ConversationGroup'
import type { GroupedConversations } from './types'
import { TIME_GROUP_LABELS } from './utils'

interface ConversationListProps {
  groupedConversations: GroupedConversations
  activeConversationId: string
  onDelete?: (id: string) => void
  hasNextPage?: boolean
  isFetchingNextPage?: boolean
  onLoadMore?: () => void
  isRefreshing?: boolean
}

export const ConversationList: FC<ConversationListProps> = ({
  groupedConversations,
  activeConversationId,
  onDelete,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  isRefreshing,
}) => {
  const loadMoreRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!hasNextPage || !onLoadMore) return

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && !isFetchingNextPage) {
          onLoadMore()
        }
      },
      { threshold: 0.1 },
    )

    const currentRef = loadMoreRef.current
    if (currentRef) {
      observer.observe(currentRef)
    }

    return () => {
      if (currentRef) {
        observer.unobserve(currentRef)
      }
    }
  }, [hasNextPage, isFetchingNextPage, onLoadMore])

  const hasConversations =
    groupedConversations.today.length > 0 ||
    groupedConversations.thisWeek.length > 0 ||
    groupedConversations.thisMonth.length > 0 ||
    groupedConversations.older.length > 0

  return (
    <main className="mt-4 flex h-full flex-1 flex-col space-y-4 overflow-y-auto">
      <div className="w-full p-3">
        {isRefreshing && (
          <div className="flex items-center justify-center gap-2 pb-3 text-muted-foreground text-xs">
            <Loader2 className="h-3 w-3 animate-spin" />
            <span>Fetching latest conversations</span>
          </div>
        )}
        {!hasConversations ? (
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <MessageSquare className="mb-3 h-10 w-10 text-muted-foreground/50" />
            <p className="text-muted-foreground text-sm">
              No conversations yet
            </p>
            <Link to="/" className="mt-2 text-primary text-sm hover:underline">
              Start a new chat
            </Link>
          </div>
        ) : (
          <>
            <ConversationGroup
              label={TIME_GROUP_LABELS.today}
              conversations={groupedConversations.today}
              onDelete={onDelete}
              activeConversationId={activeConversationId}
            />
            <ConversationGroup
              label={TIME_GROUP_LABELS.thisWeek}
              conversations={groupedConversations.thisWeek}
              onDelete={onDelete}
              activeConversationId={activeConversationId}
            />
            <ConversationGroup
              label={TIME_GROUP_LABELS.thisMonth}
              conversations={groupedConversations.thisMonth}
              onDelete={onDelete}
              activeConversationId={activeConversationId}
            />
            <ConversationGroup
              label={TIME_GROUP_LABELS.older}
              conversations={groupedConversations.older}
              onDelete={onDelete}
              activeConversationId={activeConversationId}
            />

            {hasNextPage && (
              <div
                ref={loadMoreRef}
                className="flex items-center justify-center py-4"
              >
                {isFetchingNextPage && (
                  <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                )}
              </div>
            )}
          </>
        )}
      </div>
    </main>
  )
}
