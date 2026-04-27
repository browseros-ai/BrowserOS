import {
  AlertTriangle,
  ArrowRight,
  Bot,
  ChevronDown,
  FileText,
  Folder,
  Layers,
  Loader2,
  Mic,
  Paperclip,
  RefreshCw,
  Square,
  X,
} from 'lucide-react'
import {
  type DragEvent,
  type FC,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { AppSelector } from '@/components/elements/AppSelector'
import { TabPickerPopover } from '@/components/elements/tab-picker-popover'
import { WorkspaceSelector } from '@/components/elements/workspace-selector'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { AgentEntry } from '@/entrypoints/app/agents/useOpenClaw'
import { McpServerIcon } from '@/entrypoints/app/connect-mcp/McpServerIcon'
import { useGetUserMCPIntegrations } from '@/entrypoints/app/connect-mcp/useGetUserMCPIntegrations'
import { type StagedAttachment, stageAttachments } from '@/lib/attachments'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import { useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { cn } from '@/lib/utils'
import { useVoiceInput } from '@/lib/voice/useVoiceInput'
import { useWorkspace } from '@/lib/workspace/use-workspace'
import { AgentSelector } from './AgentSelector'
import type { OutboundMessage } from './useOutboundQueue'

export interface ConversationInputSendInput {
  text: string
  attachments: StagedAttachment[]
}

interface ConversationInputProps {
  agents: AgentEntry[]
  selectedAgentId: string | null
  onSelectAgent: (agent: AgentEntry) => void
  onSend: (input: ConversationInputSendInput) => void
  onCreateAgent?: () => void
  streaming: boolean
  disabled?: boolean
  status?: string
  placeholder?: string
  variant?: 'home' | 'conversation'
  // Outbound queue: when present, the composer renders the queue strip
  // above the textarea and lets the user keep sending while a previous
  // turn is in flight. Optional so non-conversation variants (the home
  // page) can opt out — the queue only makes sense in the conversation
  // page where each enqueued message will eventually be delivered to the
  // active agent.
  outboundQueue?: OutboundMessage[]
  onCancelQueued?: (id: string) => void
  onRetryQueued?: (id: string) => void
}

function InputActionButton({
  disabled,
  onClick,
  streaming,
}: {
  disabled: boolean
  onClick: () => void
  streaming: boolean
}) {
  return (
    <Button
      onClick={onClick}
      size="icon"
      disabled={disabled}
      className="h-10 w-10 flex-shrink-0 rounded-xl bg-primary text-primary-foreground hover:bg-primary/90"
    >
      {streaming ? (
        <Loader2 className="h-5 w-5 animate-spin" />
      ) : (
        <ArrowRight className="h-5 w-5" />
      )}
    </Button>
  )
}

function VoiceButton({
  isRecording,
  isTranscribing,
  onStart,
  onStop,
}: {
  isRecording: boolean
  isTranscribing: boolean
  onStart: () => void
  onStop: () => void
}) {
  if (isRecording) {
    return (
      <Button
        type="button"
        size="icon"
        onClick={onStop}
        className="h-10 w-10 flex-shrink-0 rounded-xl bg-red-600 text-white hover:bg-red-700"
      >
        <Square className="h-4 w-4" />
      </Button>
    )
  }

  if (isTranscribing) {
    return (
      <Button
        type="button"
        variant="ghost"
        size="icon"
        disabled
        className="h-10 w-10 flex-shrink-0 rounded-xl"
      >
        <Loader2 className="h-5 w-5 animate-spin" />
      </Button>
    )
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      onClick={onStart}
      className="h-10 w-10 flex-shrink-0 rounded-xl text-muted-foreground transition-colors hover:text-foreground"
      title="Voice input"
    >
      <Mic className="h-5 w-5" />
    </Button>
  )
}

function ContextControls({
  agents,
  onCreateAgent,
  onSelectAgent,
  selectedAgentId,
  selectedTabs,
  onToggleTab,
  showAgentSelector,
  status,
  onAttachClick,
  attachDisabled,
}: {
  agents: AgentEntry[]
  onCreateAgent?: () => void
  onSelectAgent: (agent: AgentEntry) => void
  selectedAgentId: string | null
  selectedTabs: chrome.tabs.Tab[]
  onToggleTab: (tab: chrome.tabs.Tab) => void
  showAgentSelector: boolean
  status?: string
  onAttachClick: () => void
  attachDisabled: boolean
}) {
  const { supports } = useCapabilities()
  const { selectedFolder } = useWorkspace()
  const { servers: mcpServers } = useMcpServers()
  const { data: userMCPIntegrations } = useGetUserMCPIntegrations()

  const connectedManagedServers = mcpServers.filter((server) => {
    if (server.type !== 'managed' || !server.managedServerName) return false
    return userMCPIntegrations?.integrations?.find(
      (integration) => integration.name === server.managedServerName,
    )?.is_authenticated
  })

  return (
    <div className="flex items-center justify-between border-border/40 border-t px-4 py-2.5">
      <div className="flex items-center gap-1">
        {showAgentSelector ? (
          <AgentSelector
            agents={agents}
            selectedAgentId={selectedAgentId}
            onSelectAgent={onSelectAgent}
            onCreateAgent={onCreateAgent}
            status={status}
          />
        ) : null}
        {supports(Feature.WORKSPACE_FOLDER_SUPPORT) ? (
          <WorkspaceSelector>
            <Button
              variant="ghost"
              className={cn(
                'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
                'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                'data-[state=open]:bg-accent',
              )}
            >
              <Folder className="h-4 w-4" />
              <span>{selectedFolder?.name || 'Add workspace'}</span>
              <ChevronDown className="h-3 w-3" />
            </Button>
          </WorkspaceSelector>
        ) : null}
        <TabPickerPopover
          variant="selector"
          selectedTabs={selectedTabs}
          onToggleTab={onToggleTab}
        >
          <Button
            className={cn(
              'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
              selectedTabs.length > 0
                ? 'bg-[var(--accent-orange)]! text-white shadow-sm'
                : 'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
              'data-[state=open]:bg-accent',
            )}
          >
            <Layers className="h-4 w-4" />
            <span>Tabs</span>
          </Button>
        </TabPickerPopover>
        <Button
          type="button"
          variant="ghost"
          onClick={onAttachClick}
          disabled={attachDisabled}
          title="Attach files"
          className={cn(
            'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
            'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          )}
        >
          <Paperclip className="h-4 w-4" />
          <span>Attach</span>
        </Button>
      </div>

      {supports(Feature.MANAGED_MCP_SUPPORT) ? (
        <div className="ml-auto flex items-center gap-1.5">
          <AppSelector side="bottom">
            <Button
              variant="ghost"
              className={cn(
                'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
                'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                'data-[state=open]:bg-accent',
              )}
            >
              <div className="flex items-center -space-x-1.5">
                {connectedManagedServers.slice(0, 4).map((server) => (
                  <div
                    key={server.id}
                    className="rounded-full ring-2 ring-card"
                  >
                    <McpServerIcon
                      serverName={server.managedServerName ?? ''}
                      size={16}
                    />
                  </div>
                ))}
              </div>
              {connectedManagedServers.length > 4 ? (
                <span className="text-xs">
                  +{connectedManagedServers.length - 4}
                </span>
              ) : null}
              <span>Apps</span>
              <ChevronDown className="h-3 w-3" />
            </Button>
          </AppSelector>
        </div>
      ) : null}
    </div>
  )
}

function HomeShell({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-hidden rounded-[1.55rem] border border-border/60 bg-card/95 shadow-sm">
      {children}
    </div>
  )
}

function ConversationShell({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-hidden rounded-[1.35rem] border border-border/50 bg-background/95 shadow-[0_10px_30px_rgba(15,23,42,0.06)] backdrop-blur-md">
      {children}
    </div>
  )
}

export const ConversationInput: FC<ConversationInputProps> = ({
  agents,
  selectedAgentId,
  onSelectAgent,
  onSend,
  onCreateAgent,
  streaming,
  disabled,
  status,
  placeholder,
  variant = 'conversation',
  outboundQueue,
  onCancelQueued,
  onRetryQueued,
}) => {
  const [input, setInput] = useState('')
  const [selectedTabs, setSelectedTabs] = useState<chrome.tabs.Tab[]>([])
  const [isExpandedDraft, setIsExpandedDraft] = useState(false)
  const [attachments, setAttachments] = useState<StagedAttachment[]>([])
  const [attachmentError, setAttachmentError] = useState<string | null>(null)
  const [isStaging, setIsStaging] = useState(false)
  const [isDragOver, setIsDragOver] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const voice = useVoiceInput()
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const selectedAgent = agents.find(
    (agent) => agent.agentId === selectedAgentId,
  )
  const isConversation = variant === 'conversation'

  const stageFiles = async (files: File[]) => {
    if (files.length === 0) return
    setIsStaging(true)
    setAttachmentError(null)
    try {
      const result = await stageAttachments(files, attachments.length)
      if (result.staged.length > 0) {
        setAttachments((prev) => [...prev, ...result.staged])
      }
      if (result.errors.length > 0) {
        setAttachmentError(result.errors.map((e) => e.message).join(' \u2022 '))
      }
    } finally {
      setIsStaging(false)
    }
  }

  const removeAttachment = (id: string) => {
    setAttachments((prev) => prev.filter((a) => a.id !== id))
    setAttachmentError(null)
  }

  useLayoutEffect(() => {
    const element = textareaRef.current
    if (!element) return

    const maxHeight = isConversation ? 176 : 100
    const collapsedHeight = isConversation ? 56 : 72
    element.style.height = '0px'
    const nextHeight = Math.min(element.scrollHeight, maxHeight)
    element.style.height = `${nextHeight}px`
    element.style.overflowY =
      element.scrollHeight > maxHeight ? 'auto' : 'hidden'
    setIsExpandedDraft(nextHeight > collapsedHeight)
  })

  useEffect(() => {
    if (voice.transcript && !voice.isTranscribing) {
      setInput(voice.transcript)
      voice.clearTranscript()
    }
  }, [voice.transcript, voice.isTranscribing, voice])

  const toggleTab = (tab: chrome.tabs.Tab) => {
    setSelectedTabs((prev) => {
      const isSelected = prev.some((selected) => selected.id === tab.id)
      if (isSelected) {
        return prev.filter((selected) => selected.id !== tab.id)
      }
      return [...prev, tab]
    })
  }

  const hasContent = input.trim().length > 0 || attachments.length > 0
  const queueEnabled = outboundQueue !== undefined

  const handleSend = () => {
    const text = input.trim()
    // The outbound queue accepts new messages while streaming; legacy
    // direct-send callers (e.g., the home composer) keep the original
    // streaming-blocks-send semantic.
    if (disabled || isStaging) return
    if (!queueEnabled && streaming) return
    if (!text && attachments.length === 0) return
    onSend({ text, attachments })
    setInput('')
    setAttachments([])
    setAttachmentError(null)
  }

  const handlePaste = (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = event.clipboardData?.items
    if (!items) return
    const files: File[] = []
    for (const item of items) {
      if (item.kind === 'file') {
        const file = item.getAsFile()
        if (file) files.push(file)
      }
    }
    if (files.length > 0) {
      event.preventDefault()
      void stageFiles(files)
    }
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setIsDragOver(false)
    const files = Array.from(event.dataTransfer?.files ?? [])
    if (files.length > 0) {
      void stageFiles(files)
    }
  }

  const handleDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (!event.dataTransfer?.types.includes('Files')) return
    event.preventDefault()
    setIsDragOver(true)
  }

  const handleDragLeave = (event: DragEvent<HTMLDivElement>) => {
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
      return
    }
    setIsDragOver(false)
  }

  const openFilePicker = () => {
    fileInputRef.current?.click()
  }

  const handleFileInputChange = (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const files = Array.from(event.target.files ?? [])
    event.target.value = ''
    if (files.length > 0) void stageFiles(files)
  }

  const shell = variant === 'home' ? HomeShell : ConversationShell
  const Shell = shell

  return (
    <Shell>
      <section
        // Drag/drop on a region isn't a click affordance — wrap the
        // composer in a labeled <section> so the a11y rule is satisfied
        // without misrepresenting the surface as interactive.
        aria-label="Message composer"
        className={cn('relative', isDragOver && 'ring-2 ring-primary/60')}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        <input
          ref={fileInputRef}
          type="file"
          multiple
          accept="image/png,image/jpeg,image/webp,image/gif,text/*,application/json"
          className="hidden"
          onChange={handleFileInputChange}
        />
        {attachments.length > 0 || attachmentError ? (
          <AttachmentStrip
            attachments={attachments}
            onRemove={removeAttachment}
            error={attachmentError}
          />
        ) : null}
        {queueEnabled && outboundQueue && outboundQueue.length > 0 ? (
          <OutboundQueueStrip
            messages={outboundQueue}
            onCancel={onCancelQueued}
            onRetry={onRetryQueued}
          />
        ) : null}
        <div
          className={cn(
            'flex gap-3',
            variant === 'home' ? 'px-4 py-3' : 'px-4 py-3',
            isExpandedDraft ? 'items-end' : 'items-center',
          )}
        >
          <BotInputIcon variant={variant} />
          <div className="flex-1">
            <Textarea
              ref={textareaRef}
              value={input}
              onChange={(event) => setInput(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !event.shiftKey) {
                  event.preventDefault()
                  handleSend()
                }
              }}
              onPaste={handlePaste}
              rows={1}
              placeholder={
                voice.isTranscribing
                  ? 'Transcribing...'
                  : (placeholder ??
                    `Message ${selectedAgent?.name ?? 'agent'}...`)
              }
              disabled={disabled || voice.isTranscribing}
              className={cn(
                'resize-none border-none bg-transparent px-0 text-[15px] shadow-none focus-visible:ring-0',
                '[field-sizing:fixed]',
                variant === 'home'
                  ? 'min-h-[40px] py-2 leading-6'
                  : 'min-h-[40px] py-2 leading-6',
                'placeholder:text-muted-foreground/80',
              )}
            />
          </div>
          <VoiceButton
            isRecording={voice.isRecording}
            isTranscribing={voice.isTranscribing}
            onStart={() => {
              void voice.startRecording()
            }}
            onStop={() => {
              void voice.stopRecording()
            }}
          />
          <InputActionButton
            disabled={
              !hasContent ||
              isStaging ||
              !!disabled ||
              voice.isRecording ||
              voice.isTranscribing ||
              // Only block on `streaming` for the legacy direct-send path
              // (no queue). With the queue active the press always
              // succeeds — it just enqueues instead of dispatching.
              (!queueEnabled && streaming)
            }
            onClick={handleSend}
            // Spinner stays the user-facing "agent is busy" hint; with the
            // queue active we still spin while a turn is in flight.
            streaming={streaming}
          />
        </div>
        {voice.error ? (
          <div className="px-5 pb-2 text-destructive text-xs">
            {voice.error}
          </div>
        ) : null}
        <ContextControls
          agents={agents}
          onCreateAgent={onCreateAgent}
          onSelectAgent={onSelectAgent}
          selectedAgentId={selectedAgentId}
          selectedTabs={selectedTabs}
          onToggleTab={toggleTab}
          showAgentSelector={variant === 'home'}
          status={status}
          onAttachClick={openFilePicker}
          attachDisabled={attachments.length >= 10 || isStaging || !!disabled}
        />
        {isDragOver ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center rounded-[inherit] bg-background/80 font-medium text-foreground text-sm backdrop-blur-sm">
            Drop files to attach
          </div>
        ) : null}
      </section>
    </Shell>
  )
}

function OutboundQueueStrip({
  messages,
  onCancel,
  onRetry,
}: {
  messages: OutboundMessage[]
  onCancel?: (id: string) => void
  onRetry?: (id: string) => void
}) {
  return (
    <div className="border-border/40 border-b px-4 pt-3 pb-2">
      <ul className="flex flex-col gap-1">
        {messages.map((message) => (
          <OutboundQueueItem
            key={message.id}
            message={message}
            onCancel={onCancel}
            onRetry={onRetry}
          />
        ))}
      </ul>
    </div>
  )
}

function OutboundQueueItem({
  message,
  onCancel,
  onRetry,
}: {
  message: OutboundMessage
  onCancel?: (id: string) => void
  onRetry?: (id: string) => void
}) {
  const preview = message.text.trim() || '(attachments only)'
  return (
    <li className="flex items-center gap-2 rounded-md px-2 py-1 text-xs">
      <OutboundQueueStatusIcon status={message.status} />
      <span className="min-w-0 flex-1 truncate text-muted-foreground">
        {preview}
      </span>
      {message.attachmentPreviews.length > 0 ? (
        <span className="inline-flex items-center gap-1 text-muted-foreground/70">
          <Paperclip className="size-3" />
          <span className="tabular-nums">
            {message.attachmentPreviews.length}
          </span>
        </span>
      ) : null}
      {message.status === 'queued' && onCancel ? (
        <button
          type="button"
          onClick={() => onCancel(message.id)}
          className="ml-1 inline-flex size-5 items-center justify-center rounded-full text-muted-foreground hover:bg-accent hover:text-foreground"
          aria-label="Cancel queued message"
          title="Cancel"
        >
          <X className="size-3" />
        </button>
      ) : null}
      {message.status === 'failed' ? (
        <span className="ml-1 inline-flex items-center gap-2 text-destructive">
          <span className="max-w-[160px] truncate" title={message.error}>
            {message.error ?? 'Failed'}
          </span>
          {onRetry ? (
            <button
              type="button"
              onClick={() => onRetry(message.id)}
              className="inline-flex size-5 items-center justify-center rounded-full hover:bg-accent hover:text-foreground"
              aria-label="Retry failed message"
              title="Retry"
            >
              <RefreshCw className="size-3" />
            </button>
          ) : null}
          {onCancel ? (
            <button
              type="button"
              onClick={() => onCancel(message.id)}
              className="inline-flex size-5 items-center justify-center rounded-full hover:bg-accent hover:text-foreground"
              aria-label="Discard failed message"
              title="Discard"
            >
              <X className="size-3" />
            </button>
          ) : null}
        </span>
      ) : null}
    </li>
  )
}

function OutboundQueueStatusIcon({
  status,
}: {
  status: OutboundMessage['status']
}) {
  if (status === 'sending') {
    return (
      <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
    )
  }
  if (status === 'failed') {
    return <AlertTriangle className="size-3.5 shrink-0 text-destructive" />
  }
  return (
    <span className="inline-block size-2 shrink-0 rounded-full bg-muted-foreground/40" />
  )
}

function AttachmentStrip({
  attachments,
  onRemove,
  error,
}: {
  attachments: StagedAttachment[]
  onRemove: (id: string) => void
  error: string | null
}) {
  return (
    <div className="border-border/40 border-b px-4 pt-3 pb-2">
      {attachments.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {attachments.map((attachment) => (
            <AttachmentChip
              key={attachment.id}
              attachment={attachment}
              onRemove={() => onRemove(attachment.id)}
            />
          ))}
        </div>
      ) : null}
      {error ? (
        <div className="mt-2 text-destructive text-xs">{error}</div>
      ) : null}
    </div>
  )
}

function AttachmentChip({
  attachment,
  onRemove,
}: {
  attachment: StagedAttachment
  onRemove: () => void
}) {
  if (attachment.kind === 'image' && attachment.dataUrl) {
    return (
      <div className="group relative size-16 overflow-hidden rounded-md border border-border/60">
        <img
          src={attachment.dataUrl}
          alt={attachment.name}
          className="size-full object-cover"
        />
        <button
          type="button"
          onClick={onRemove}
          className="absolute top-1 right-1 inline-flex size-5 items-center justify-center rounded-full bg-background/80 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100"
          aria-label={`Remove ${attachment.name}`}
        >
          <X className="size-3" />
        </button>
      </div>
    )
  }
  return (
    <div className="group flex max-w-[220px] items-center gap-2 rounded-md border border-border/60 bg-background/60 px-2 py-1.5">
      <FileText className="size-4 shrink-0 text-muted-foreground" />
      <span className="truncate text-xs">{attachment.name}</span>
      <button
        type="button"
        onClick={onRemove}
        className="ml-1 inline-flex size-4 items-center justify-center text-muted-foreground hover:text-foreground"
        aria-label={`Remove ${attachment.name}`}
      >
        <X className="size-3" />
      </button>
    </div>
  )
}

function BotInputIcon({ variant }: { variant: 'home' | 'conversation' }) {
  return (
    <div
      className={cn(
        'flex items-center justify-center text-[var(--accent-orange)]',
        variant === 'home'
          ? 'h-8 w-8 rounded-lg bg-[var(--accent-orange)]/10'
          : 'h-8 w-8 rounded-lg bg-[var(--accent-orange)]/10',
      )}
    >
      <Bot className="h-4 w-4" />
    </div>
  )
}
