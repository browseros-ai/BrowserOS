// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BackendNodeId } from './dom'
import type { FrameId } from './page'
import type { RemoteObject, StackTrace } from './runtime'

// ══ Types ══

export interface Annotation {
  readOnly?: boolean
  untrustedContent?: boolean
  consequential?: boolean
  autosubmit?: boolean
}

export type InvocationStatus = 'Completed' | 'Canceled' | 'Error'

export interface Tool {
  name: string
  description: string
  inputSchema?: Record<string, unknown>
  annotations?: Annotation
  frameId: FrameId
  backendNodeId?: BackendNodeId
  stackTrace?: StackTrace
}

export interface RemovedTool {
  name: string
  frameId: FrameId
}

// ══ Commands ══

export interface InvokeToolParams {
  frameId: FrameId
  toolName: string
  input: Record<string, unknown>
}

export interface InvokeToolResult {
  invocationId: string
}

export interface CancelInvocationParams {
  invocationId: string
}

// ══ Events ══

export interface ToolsAddedEvent {
  tools: Tool[]
}

export interface ToolsRemovedEvent {
  tools: RemovedTool[]
}

export interface ToolInvokedEvent {
  toolName: string
  frameId: FrameId
  invocationId: string
  input: string
}

export interface ToolRespondedEvent {
  invocationId: string
  status: InvocationStatus
  output?: unknown
  errorText?: string
  exception?: RemoteObject
}
