import { Loader2, MessageSquare, Trash2 } from 'lucide-react'
import { type FC, useEffect, useRef, useState } from 'react'
import { Link } from 'react-router'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
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
  onClearAll?: () => void
  isClearingAll?: boolean
}

export const ConversationList: FC<ConversationListProps> = ({
  groupedConversations,
  activeConversationId,
  onDelete,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  isRefreshing,
  onClearAll,
  isClearingAll,
}) => {
  const loadMoreRef = useRef<HTMLDivElement>(null)
  const [showClearAllDialog, setShowClearAllDialog] = useState(false)

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

  const handleConfirmClearAll = () => {
    onClearAll?.()
    setShowClearAllDialog(false)
  }

  return (
    <>
      <main className="mt-4 flex h-full flex-1 flex-col space-y-4 overflow-y-auto">
        <div className="w-full p-3">
          <div className="mb-3 flex items-center justify-between gap-3 px-1">
            <h2 className="font-semibold text-sm">Chat history</h2>
            {onClearAll && hasConversations && (
              <button
                type="button"
                onClick={() => setShowClearAllDialog(true)}
                disabled={isClearingAll}
                className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2.5 font-medium text-muted-foreground text-xs transition-colors hover:bg-destructive/10 hover:text-destructive disabled:pointer-events-none disabled:opacity-50"
                title="Clear sessions"
              >
                {isClearingAll ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Trash2 className="h-3.5 w-3.5" />
                )}
                Clear sessions
              </button>
            )}
          </div>

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
              <Link
                to="/"
                className="mt-2 text-primary text-sm hover:underline"
              >
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

      <AlertDialog
        open={showClearAllDialog}
        onOpenChange={setShowClearAllDialog}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Clear all sessions?</AlertDialogTitle>
            <AlertDialogDescription>
              This action permanently deletes every chat session in history.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleConfirmClearAll}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Clear sessions
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
