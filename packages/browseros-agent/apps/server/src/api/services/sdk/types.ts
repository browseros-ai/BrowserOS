/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * SDK Types - Type definitions and request schemas for SDK services
 */

import { BrowserContextSchema } from '@browseros/shared/schemas/browser-context'
import { ChatSourceSchema } from '@browseros/shared/schemas/chat-source'
import { LLMConfigSchema } from '@browseros/shared/schemas/llm'
import { z } from 'zod'
import type { Browser } from '../../../browser/browser'

// Request validation schemas

export const NavRequestSchema = z.object({
  url: z.string().url(),
  tabId: z.number().optional(),
  windowId: z.number().optional(),
})

export const ActRequestSchema = z.object({
  instruction: z.string().min(1),
  context: z.record(z.unknown()).optional(),
  maxSteps: z.number().optional(),
  browserContext: BrowserContextSchema.optional(),
  source: ChatSourceSchema.optional(),
  llm: LLMConfigSchema.optional(),
  /** Session ID for state persistence. If provided, reuses existing session. */
  sessionId: z.string().uuid().optional(),
})

export const ExtractRequestSchema = z.object({
  instruction: z.string().min(1),
  schema: z.record(z.unknown()),
  context: z.record(z.unknown()).optional(),
  windowId: z.number().optional(),
  tabId: z.number().optional(),
})

export const VerifyRequestSchema = z.object({
  expectation: z.string().min(1),
  context: z.record(z.unknown()).optional(),
  windowId: z.number().optional(),
  tabId: z.number().optional(),
  llm: LLMConfigSchema.optional(),
})

export type NavRequest = z.infer<typeof NavRequestSchema>
export type ActRequest = z.infer<typeof ActRequestSchema>
export type ExtractRequest = z.infer<typeof ExtractRequestSchema>
export type VerifyRequest = z.infer<typeof VerifyRequestSchema>

// Shared types

export interface SdkDeps {
  port: number
  browser: Browser
  browserosId?: string
}

export interface ActiveTab {
  tabId: number
  url: string
  title: string
  windowId: number
}

export interface Screenshot {
  data: string
  mimeType: string
}

export interface InteractiveElements {
  content: string
}

export interface NavigateResult {
  tabId: number
  windowId: number
}

export interface PageLoadStatus {
  tabId: number
  isDOMContentLoaded: boolean
  isResourcesLoading: boolean
  isPageComplete: boolean
}

export class SdkError extends Error {
  constructor(
    message: string,
    public statusCode: number = 500,
  ) {
    super(message)
    this.name = 'SdkError'
  }
}
