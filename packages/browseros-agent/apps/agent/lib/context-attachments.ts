export type ContextAttachmentKind = 'file' | 'memory'

export interface ContextAttachment {
  id: string
  kind: ContextAttachmentKind
  title: string
  source?: string
  content: string
  truncated?: boolean
}
