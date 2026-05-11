import { Brain, Check, Loader2, Pencil, Plus, RefreshCw, X } from 'lucide-react'
import type { FC } from 'react'
import { useState } from 'react'
import { MessageResponse } from '@/components/ai-elements/message'
import { Button } from '@/components/ui/button'
import { MarkdownEditor } from '@/components/ui/MarkdownEditor'
import { useMemoryContent } from './useMemoryContent'

export const MemoryViewer: FC = () => {
  const {
    content,
    isLoading,
    error,
    refetch,
    save,
    isSaving,
    saveError,
    resetSaveError,
  } = useMemoryContent()
  const [isEditing, setIsEditing] = useState(false)
  const [editContent, setEditContent] = useState('')

  const handleEdit = () => {
    resetSaveError()
    setEditContent(content || '')
    setIsEditing(true)
  }

  const handleCreate = () => {
    resetSaveError()
    setEditContent('')
    setIsEditing(true)
  }

  const handleCancel = () => {
    resetSaveError()
    setIsEditing(false)
    setEditContent('')
  }

  const handleSave = async () => {
    try {
      await save(editContent)
      setIsEditing(false)
    } catch {
      // saveError is rendered inline below the textarea
    }
  }

  if (isLoading) {
    return (
      <div className="rounded-xl border border-border bg-card p-6">
        <div className="flex flex-col items-center gap-3 py-6 text-center">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          <div>
            <p className="font-medium text-sm">Loading your memory...</p>
            <p className="mt-1 text-muted-foreground text-xs">
              Fetching your latest CORE.md from the Shimmy server.
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="mt-2 gap-1.5"
            onClick={() => refetch()}
          >
            <RefreshCw className="h-3.5 w-3.5" />
            Retry
          </Button>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="rounded-xl border border-destructive/50 bg-destructive/5 p-6">
        <p className="text-destructive text-sm">
          Could not load memory. Shimmy server may be disconnected.
        </p>
        <p className="mt-1 text-muted-foreground text-xs">
          Try restarting Shimmy Sup/server, then click retry.
        </p>
        <Button
          variant="outline"
          size="sm"
          className="mt-3 gap-1.5"
          onClick={() => refetch()}
        >
          <RefreshCw className="h-3.5 w-3.5" />
          Retry
        </Button>
      </div>
    )
  }

  if (!content && !isEditing) {
    return (
      <div className="rounded-xl border border-border bg-card p-6">
        <div className="flex flex-col items-center gap-3 py-8 text-center">
          <Brain className="h-8 w-8 text-muted-foreground/50" />
          <div>
            <p className="font-medium text-sm">No memories yet</p>
            <p className="mt-1 text-muted-foreground text-xs">
              Start a conversation and your Sup will learn about you, or add
              memories directly.
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="mt-2 gap-1.5"
            onClick={handleCreate}
          >
            <Plus className="h-3.5 w-3.5" />
            Add memory
          </Button>
        </div>
      </div>
    )
  }

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: mouseEnter for background refetch
    <div
      className="rounded-xl border border-border bg-card shadow-sm"
      onMouseEnter={() => {
        if (!isEditing) refetch()
      }}
    >
      <div className="flex flex-col gap-3 border-border border-b px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-2">
          <Brain className="h-4 w-4 text-muted-foreground" />
          <span className="font-medium text-sm">CORE.md</span>
          {isEditing ? (
            <span className="shrink-0 text-violet-500 text-xs">editing</span>
          ) : (
            <span className="shrink-0 text-muted-foreground text-xs">
              editable
            </span>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {isEditing ? (
            <>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 gap-1 px-2 text-xs"
                onClick={handleCancel}
                disabled={isSaving}
              >
                <X className="h-3 w-3" />
                Cancel
              </Button>
              <Button
                size="sm"
                className="h-7 gap-1 px-2 text-xs"
                onClick={handleSave}
                disabled={isSaving}
              >
                {isSaving ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <Check className="h-3 w-3" />
                )}
                Save
              </Button>
            </>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1 px-2 text-xs"
              onClick={handleEdit}
            >
              <Pencil className="h-3 w-3" />
              Edit
            </Button>
          )}
        </div>
      </div>

      {isEditing ? (
        <div className="space-y-2 p-3">
          <MarkdownEditor
            value={editContent}
            onChange={setEditContent}
            className="styled-scrollbar max-h-[480px] min-h-[200px] overflow-y-auto text-sm"
            placeholder="Write facts about yourself — name, preferences, projects, tools..."
            autoFocus
          />
          {saveError && (
            <p className="text-destructive text-xs">
              {saveError.message || 'Failed to save. Please try again.'}
            </p>
          )}
        </div>
      ) : (
        <div className="prose prose-sm dark:prose-invert [&_[data-streamdown='code-block']]:!w-full [&_[data-streamdown='table-wrapper']]:!w-full styled-scrollbar max-h-[65vh] max-w-none overflow-auto break-words p-4">
          <MessageResponse>{content ?? ''}</MessageResponse>
        </div>
      )}
    </div>
  )
}
