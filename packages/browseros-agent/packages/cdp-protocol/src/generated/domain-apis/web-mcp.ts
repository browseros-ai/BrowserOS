// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  ToolInvokedEvent,
  ToolRespondedEvent,
  ToolsAddedEvent,
  ToolsRemovedEvent,
} from '../domains/web-mcp'

export interface WebMCPApi {
  // ── Commands ──

  enable(): Promise<void>
  disable(): Promise<void>

  // ── Events ──

  on(
    event: 'toolsAdded',
    handler: (params: ToolsAddedEvent) => void,
  ): () => void
  on(
    event: 'toolsRemoved',
    handler: (params: ToolsRemovedEvent) => void,
  ): () => void
  on(
    event: 'toolInvoked',
    handler: (params: ToolInvokedEvent) => void,
  ): () => void
  on(
    event: 'toolResponded',
    handler: (params: ToolRespondedEvent) => void,
  ): () => void
}
