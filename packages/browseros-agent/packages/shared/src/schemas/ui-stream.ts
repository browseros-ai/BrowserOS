/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * UI Message Stream schema (Vercel AI SDK format) - single source of truth.
 * Use z.infer<> for TypeScript types.
 */

import { z } from 'zod'

// Individual event schemas with explicit type annotations for isolatedDeclarations
const StartEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'start'>
  messageId: z.ZodOptional<z.ZodString>
}> = z.object({
  type: z.literal('start'),
  messageId: z.string().optional(),
})

const StartStepEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'start-step'>
}> = z.object({ type: z.literal('start-step') })

const FinishStepEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'finish-step'>
}> = z.object({ type: z.literal('finish-step') })

const FinishEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'finish'>
  finishReason: z.ZodString
  messageMetadata: z.ZodOptional<z.ZodUnknown>
}> = z.object({
  type: z.literal('finish'),
  finishReason: z.string(),
  messageMetadata: z.unknown().optional(),
})

const AbortEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'abort'>
}> = z.object({ type: z.literal('abort') })

const ErrorEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'error'>
  errorText: z.ZodString
}> = z.object({
  type: z.literal('error'),
  errorText: z.string(),
})

const TextStartEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'text-start'>
  id: z.ZodString
}> = z.object({
  type: z.literal('text-start'),
  id: z.string(),
})

const TextDeltaEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'text-delta'>
  id: z.ZodString
  delta: z.ZodString
}> = z.object({
  type: z.literal('text-delta'),
  id: z.string(),
  delta: z.string(),
})

const TextEndEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'text-end'>
  id: z.ZodString
}> = z.object({
  type: z.literal('text-end'),
  id: z.string(),
})

const ReasoningStartEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'reasoning-start'>
  id: z.ZodString
}> = z.object({
  type: z.literal('reasoning-start'),
  id: z.string(),
})

const ReasoningDeltaEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'reasoning-delta'>
  id: z.ZodString
  delta: z.ZodString
}> = z.object({
  type: z.literal('reasoning-delta'),
  id: z.string(),
  delta: z.string(),
})

const ReasoningEndEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'reasoning-end'>
  id: z.ZodString
}> = z.object({
  type: z.literal('reasoning-end'),
  id: z.string(),
})

const ToolInputStartEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'tool-input-start'>
  toolCallId: z.ZodString
  toolName: z.ZodString
}> = z.object({
  type: z.literal('tool-input-start'),
  toolCallId: z.string(),
  toolName: z.string(),
})

const ToolInputDeltaEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'tool-input-delta'>
  toolCallId: z.ZodString
  inputTextDelta: z.ZodString
}> = z.object({
  type: z.literal('tool-input-delta'),
  toolCallId: z.string(),
  inputTextDelta: z.string(),
})

const ToolInputAvailableEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'tool-input-available'>
  toolCallId: z.ZodString
  toolName: z.ZodString
  input: z.ZodUnknown
}> = z.object({
  type: z.literal('tool-input-available'),
  toolCallId: z.string(),
  toolName: z.string(),
  input: z.unknown(),
})

const ToolInputErrorEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'tool-input-error'>
  toolCallId: z.ZodString
  errorText: z.ZodString
}> = z.object({
  type: z.literal('tool-input-error'),
  toolCallId: z.string(),
  errorText: z.string(),
})

const ToolOutputAvailableEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'tool-output-available'>
  toolCallId: z.ZodString
  output: z.ZodUnknown
}> = z.object({
  type: z.literal('tool-output-available'),
  toolCallId: z.string(),
  output: z.unknown(),
})

const ToolOutputErrorEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'tool-output-error'>
  toolCallId: z.ZodString
  errorText: z.ZodString
}> = z.object({
  type: z.literal('tool-output-error'),
  toolCallId: z.string(),
  errorText: z.string(),
})

const SourceUrlEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'source-url'>
  sourceId: z.ZodString
  url: z.ZodString
  title: z.ZodOptional<z.ZodString>
}> = z.object({
  type: z.literal('source-url'),
  sourceId: z.string(),
  url: z.string(),
  title: z.string().optional(),
})

const FileEventSchema: z.ZodObject<{
  type: z.ZodLiteral<'file'>
  url: z.ZodString
  mediaType: z.ZodString
}> = z.object({
  type: z.literal('file'),
  url: z.string(),
  mediaType: z.string(),
})

/**
 * Zod schema for UIMessageStreamEvent validation.
 * The type is derived from this schema - single source of truth.
 */
export const UIMessageStreamEventSchema = z.discriminatedUnion('type', [
  StartEventSchema,
  StartStepEventSchema,
  FinishStepEventSchema,
  FinishEventSchema,
  AbortEventSchema,
  ErrorEventSchema,
  TextStartEventSchema,
  TextDeltaEventSchema,
  TextEndEventSchema,
  ReasoningStartEventSchema,
  ReasoningDeltaEventSchema,
  ReasoningEndEventSchema,
  ToolInputStartEventSchema,
  ToolInputDeltaEventSchema,
  ToolInputAvailableEventSchema,
  ToolInputErrorEventSchema,
  ToolOutputAvailableEventSchema,
  ToolOutputErrorEventSchema,
  SourceUrlEventSchema,
  FileEventSchema,
])

/**
 * UI Message Stream events (Vercel AI SDK format).
 * Derived from UIMessageStreamEventSchema.
 */
export type UIMessageStreamEvent = z.infer<typeof UIMessageStreamEventSchema>
