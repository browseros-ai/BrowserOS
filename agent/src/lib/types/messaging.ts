import { z } from 'zod'

/**
 * Message types for extension communication between background, content, and sidepanel
 */
export enum MessageType {
  LOG = 'LOG',
  WORKFLOW_STATUS = 'WORKFLOW_STATUS',
  EXECUTE_QUERY = 'EXECUTE_QUERY',
  HEARTBEAT = 'HEARTBEAT',
  HEARTBEAT_ACK = 'HEARTBEAT_ACK',
  AGENT_STREAM_UPDATE = 'AGENT_STREAM_UPDATE',
  CANCEL_TASK = 'CANCEL_TASK',
  CLOSE_PANEL = 'CLOSE_PANEL',
  RESET_CONVERSATION = 'RESET_CONVERSATION',
  GLOW_START = 'GLOW_START',
  GLOW_STOP = 'GLOW_STOP',
  MCP_INSTALL_SERVER = 'MCP_INSTALL_SERVER',
  MCP_SERVER_STATUS = 'MCP_SERVER_STATUS',
  MCP_GET_INSTALLED_SERVERS = 'MCP_GET_INSTALLED_SERVERS',
  MCP_DELETE_SERVER = 'MCP_DELETE_SERVER',
  HUMAN_INPUT_RESPONSE = 'HUMAN_INPUT_RESPONSE',
  PLAN_EDIT_RESPONSE = 'PLAN_EDIT_RESPONSE',
  GENERATE_PLAN = 'GENERATE_PLAN',
  REFINE_PLAN = 'REFINE_PLAN',
  PLAN_GENERATION_UPDATE = 'PLAN_GENERATION_UPDATE',
  // MCP related
  GET_MCP_SERVERS = 'GET_MCP_SERVERS',
  CONNECT_MCP_SERVER = 'CONNECT_MCP_SERVER',
  DISCONNECT_MCP_SERVER = 'DISCONNECT_MCP_SERVER',
  CALL_MCP_TOOL = 'CALL_MCP_TOOL',
  // Logging
  LOG_MESSAGE = 'LOG_MESSAGE',
  LOG_METRIC = 'LOG_METRIC',
  // Newtab to sidepanel communication
  EXECUTE_IN_SIDEPANEL = 'EXECUTE_IN_SIDEPANEL',
  EXECUTION_STARTING = 'EXECUTION_STARTING',
  // LLM Provider management
  GET_LLM_PROVIDERS = 'GET_LLM_PROVIDERS',
  SAVE_LLM_PROVIDERS = 'SAVE_LLM_PROVIDERS'
}

// Create a zod enum for MessageType
const MessageTypeSchema = z.nativeEnum(MessageType)

/**
 * Base message schema
 */
export const MessageSchema = z.object({
  type: MessageTypeSchema,
  payload: z.unknown()
})

export type Message = z.infer<typeof MessageSchema>


/**
 * Log message schema
 */
export const LogMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.LOG),
  payload: z.object({
    source: z.string(),
    message: z.string(),
    level: z.enum(['info', 'error', 'warning']),
    timestamp: z.string()
  })
})

export type LogMessage = z.infer<typeof LogMessageSchema>


/**
 * Workflow status message schema
 */
export const WorkflowStatusMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.WORKFLOW_STATUS),
  payload: z.object({
    workflowId: z.string(),
    steps: z.array(
      z.object({
        id: z.string(),
        status: z.string(),
        message: z.string().optional(),
        error: z.string().optional()
      })
    ),
    output: z.unknown().optional()
  })
})

export type WorkflowStatusMessage = z.infer<typeof WorkflowStatusMessageSchema>


/**
 * Execution metadata schema for query execution
 */
export const ExecutionMetadataSchema = z.object({
  source: z.enum(['newtab', 'sidepanel', 'popup']).optional(),  // Source of the query
})

export type ExecutionMetadata = z.infer<typeof ExecutionMetadataSchema>

/**
 * Execute query message schema
 */
export const ExecuteQueryMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.EXECUTE_QUERY),
  payload: z.object({
    query: z.string(),
    tabIds: z.array(z.number()).optional(),
    chatMode: z.boolean().optional(),
    metadata: ExecutionMetadataSchema.optional()
  })
})

export type ExecuteQueryMessage = z.infer<typeof ExecuteQueryMessageSchema>

/**
 * Heartbeat message schema
 */
export const HeartbeatMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.HEARTBEAT),
  payload: z.object({
    timestamp: z.number()  // Timestamp when heartbeat was sent
  })
})

export type HeartbeatMessage = z.infer<typeof HeartbeatMessageSchema>

/**
 * Heartbeat acknowledgment message schema
 */
export const HeartbeatAckMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.HEARTBEAT_ACK),
  payload: z.object({
    timestamp: z.number()  // Original timestamp from heartbeat
  })
})

export type HeartbeatAckMessage = z.infer<typeof HeartbeatAckMessageSchema>

/**
 * Agent stream update message schema
 */
export const AgentStreamUpdateMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.AGENT_STREAM_UPDATE),
  payload: z.object({
    step: z.number(),  // Current step number
    action: z.string(),  // What the agent is doing
    status: z.enum(['thinking', 'executing', 'completed', 'error', 'debug']),  // Status of the current step
    details: z.object({
      content: z.string().optional(),  // Agent's thinking or response
      toolName: z.string().optional(),  // Tool being used
      toolArgs: z.any().optional(),  // Arguments passed to the tool
      toolResult: z.string().optional(),  // Result from tool execution
      error: z.string().optional(),  // Error message if any
      messageType: z.string().optional(),  // Type of message (ToolCall, ToolResponse, etc.)
      messageId: z.string().optional(),  // ID for tracking streaming messages
      segmentId: z.number().optional(),  // Segment ID for grouping related content
      data: z.any().optional(),  // Optional data for debug messages
      timestamp: z.string().optional()  // Optional timestamp for debug messages
    })
  })
})

export type AgentStreamUpdateMessage = z.infer<typeof AgentStreamUpdateMessageSchema>

/**
 * Cancel task message schema
 */
export const CancelTaskMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.CANCEL_TASK),
  payload: z.object({
    reason: z.string().optional(),  // Optional reason for cancellation
    source: z.string().optional()  // Source that requested cancellation (e.g., 'sidepanel', 'newtab')
  })
})

export type CancelTaskMessage = z.infer<typeof CancelTaskMessageSchema>


/**
 * Close panel message schema
 */
export const ClosePanelMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.CLOSE_PANEL),
  payload: z.object({
    reason: z.string().optional()  // Optional reason for closing
  })
})

export type ClosePanelMessage = z.infer<typeof ClosePanelMessageSchema>

/**
 * Reset conversation message schema
 */
export const ResetConversationMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.RESET_CONVERSATION),
  payload: z.object({
    source: z.string().optional()  // Source that requested reset (e.g., 'sidepanel', 'options')
  })
})

export type ResetConversationMessage = z.infer<typeof ResetConversationMessageSchema>



/**
 * Glow start message schema
 */
export const GlowStartMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.GLOW_START),
  payload: z.object({
    tabId: z.number()  // Tab ID to start glow on
  })
})

export type GlowStartMessage = z.infer<typeof GlowStartMessageSchema>

/**
 * Glow stop message schema
 */
export const GlowStopMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.GLOW_STOP),
  payload: z.object({
    tabId: z.number()  // Tab ID to stop glow on
  })
})

export type GlowStopMessage = z.infer<typeof GlowStopMessageSchema>

/**
 * Plan generation: request to generate a plan
 */
export const GeneratePlanMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.GENERATE_PLAN),
  payload: z.object({
    input: z.string(),  // Goal or description text
    context: z.string().optional(),  // Optional extra context
    maxSteps: z.number().int().positive().optional()  // Optional cap on steps
  })
})

export type GeneratePlanMessage = z.infer<typeof GeneratePlanMessageSchema>

/**
 * Plan refinement: refine an existing plan with feedback
 */
export const RefinePlanMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.REFINE_PLAN),
  payload: z.object({
    currentPlan: z.object({
      goal: z.string().optional(),
      steps: z.array(z.string()).default([])
    }),
    feedback: z.string(),  // User feedback or refinement notes
    maxSteps: z.number().int().positive().optional()
  })
})

export type RefinePlanMessage = z.infer<typeof RefinePlanMessageSchema>

/**
 * Plan generation updates (status + optional result)
 */
export const PlanGenerationUpdateMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.PLAN_GENERATION_UPDATE),
  payload: z.object({
    status: z.enum(['queued', 'started', 'thinking', 'done', 'error']),
    content: z.string().optional(), // Human-readable update
    plan: z
      .object({
        goal: z.string().optional(),
        name: z.string().optional(),
        steps: z.array(z.string())
      })
      .optional(),
    structured: z
      .object({
        steps: z.array(
          z.object({ action: z.string(), reasoning: z.string() })
        ),
        goal: z.string().optional(),
        name: z.string().optional()
      })
      .optional(),
    error: z.string().optional()
  })
})

export type PlanGenerationUpdateMessage = z.infer<typeof PlanGenerationUpdateMessageSchema>

/**
 * Get LLM providers message schema
 */
export const GetLLMProvidersMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.GET_LLM_PROVIDERS),
  payload: z.object({})
})

export type GetLLMProvidersMessage = z.infer<typeof GetLLMProvidersMessageSchema>

/**
 * Save LLM providers message schema
 */
export const SaveLLMProvidersMessageSchema = MessageSchema.extend({
  type: z.literal(MessageType.SAVE_LLM_PROVIDERS),
  payload: z.object({
    providersConfig: z.any()  // NemoProvidersConfig type
  })
})

export type SaveLLMProvidersMessage = z.infer<typeof SaveLLMProvidersMessageSchema>


/**
 * Union of all message types
 */
export const ExtensionMessageSchema = z.discriminatedUnion('type', [
  LogMessageSchema,
  WorkflowStatusMessageSchema,
  ExecuteQueryMessageSchema,
  HeartbeatMessageSchema,
  HeartbeatAckMessageSchema,
  AgentStreamUpdateMessageSchema,
  CancelTaskMessageSchema,
  ClosePanelMessageSchema,
  ResetConversationMessageSchema,
  GlowStartMessageSchema,
  GlowStopMessageSchema,
  GeneratePlanMessageSchema,
  RefinePlanMessageSchema,
  PlanGenerationUpdateMessageSchema,
  GetLLMProvidersMessageSchema,
  SaveLLMProvidersMessageSchema
])

export type ExtensionMessage = z.infer<typeof ExtensionMessageSchema>
