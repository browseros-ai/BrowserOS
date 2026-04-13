import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import type { ToolApprovalCategoryId } from '@browseros/shared/constants/tool-approval'
import type { AclRule } from '@browseros/shared/types/acl'
import type { z } from 'zod'
import type { Browser } from '../browser/browser'
import { ToolResponse, type ToolResult } from './response'

export interface ToolDefinition {
  name: string
  description: string
  approvalCategory: ToolApprovalCategoryId
  input: z.ZodType
  output?: z.ZodType
  handler: ToolHandler
}

export type ToolHandler = (
  args: unknown,
  ctx: ToolContext,
  response: ToolResponse,
) => Promise<void>

export interface ToolDirectories {
  workingDir?: string
  resourcesDir?: string
}

export interface ToolSessionContext {
  origin?: 'sidepanel' | 'newtab'
  originPageId?: number
}

export type ToolContext = {
  browser: Browser
  directories: ToolDirectories
  session?: ToolSessionContext
  aclRules?: AclRule[]
}

export function resolveWorkingPath(
  ctx: ToolContext,
  targetPath: string,
  cwd?: string,
): string {
  return resolve(cwd ?? ctx.directories.workingDir ?? tmpdir(), targetPath)
}

export function defineTool<
  TInput extends z.ZodType,
  TOutput extends z.ZodType | undefined = undefined,
>(config: {
  name: string
  description: string
  approvalCategory: ToolApprovalCategoryId
  input: TInput
  output?: TOutput
  handler: (
    args: z.infer<TInput>,
    ctx: ToolContext,
    response: ToolResponse,
  ) => Promise<void>
}): ToolDefinition {
  return config as ToolDefinition
}

export function defineToolWithCategory(
  approvalCategory: ToolApprovalCategoryId,
) {
  return <
    TInput extends z.ZodType,
    TOutput extends z.ZodType | undefined = undefined,
  >(config: {
    name: string
    description: string
    input: TInput
    output?: TOutput
    handler: (
      args: z.infer<TInput>,
      ctx: ToolContext,
      response: ToolResponse,
    ) => Promise<void>
  }): ToolDefinition =>
    defineTool({
      approvalCategory,
      ...config,
    })
}

export async function executeTool(
  tool: ToolDefinition,
  args: unknown,
  ctx: ToolContext,
  signal: AbortSignal,
): Promise<ToolResult> {
  const response = new ToolResponse()

  if (signal.aborted) {
    response.error('Request was aborted')
    return response.toResult()
  }

  if (ctx.aclRules?.length) {
    const { checkAcl } = await import('./acl/acl-guard')
    const check = await checkAcl(
      tool.name,
      args as Record<string, unknown>,
      ctx.browser,
      ctx.aclRules,
    )
    if (check.blocked) {
      const desc =
        check.rule?.description ??
        check.rule?.textMatch ??
        check.rule?.sitePattern ??
        'ACL rule'
      if (check.pageId !== undefined && check.elementId !== undefined) {
        await ctx.browser.highlightBlockedElement(
          check.pageId,
          check.elementId,
          desc,
        )
      }
      response.error(
        `Action blocked by ACL rule: "${desc}". The element on this page is restricted. Choose a different action or skip this step.`,
      )
      return response.toResult()
    }
  }

  try {
    await tool.handler(args, ctx, response)
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    response.error(`Internal error in ${tool.name}: ${message}`)
  }

  const result = await response.build(ctx.browser)

  const pageId = (args as Record<string, unknown>).page
  if (typeof pageId === 'number') {
    const tabId = ctx.browser.getTabIdForPage(pageId)
    if (tabId !== undefined) {
      result.metadata = { ...result.metadata, tabId }
    }
  }

  return result
}
