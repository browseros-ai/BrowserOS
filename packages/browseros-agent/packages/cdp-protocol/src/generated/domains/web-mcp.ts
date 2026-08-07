// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BackendNodeId } from './dom'
import type { FrameId } from './page'
import type { RemoteObject, StackTrace } from './runtime'

// ══ Types ══

export interface Annotation {
  readOnly?: boolean
  autosubmit?: boolean
}

export type InvocationStatus = 'Success' | 'Canceled' | 'Error'

export interface Tool {
  name: string
  description: string
  inputSchema?: Record<string, unknown>
  annotations?: Annotation
  frameId: FrameId
  backendNodeId?: BackendNodeId
  stackTrace?: StackTrace
}

// ══ Commands ══

// ══ Events ══

export interface ToolsAddedEvent {
  tools: Tool[]
}

export interface ToolsRemovedEvent {
  tools: Tool[]
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
