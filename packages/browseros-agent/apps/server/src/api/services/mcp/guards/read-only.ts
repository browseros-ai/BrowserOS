import {
  errorResult,
  type ToolResult,
} from '@browseros/browser-mcp/tools/framework'
import type { BrowserToolGuard } from '../browser-tool-dispatch'

/** Enforces read-only mode even if a caller invokes a registered tool directly. */
export const guardReadOnly: BrowserToolGuard = (call) =>
  guardReadOnlyCall(
    call.readOnly,
    call.tool.name,
    call.tool.annotations,
    call.args,
  )

export function guardReadOnlyCall(
  readOnly: boolean,
  toolName: string,
  annotations: { readOnlyHint?: boolean } | undefined,
  args: Record<string, unknown>,
): ToolResult | null {
  if (!readOnly) return null
  if (toolName !== 'tabs') {
    return annotations?.readOnlyHint
      ? null
      : errorResult(`${toolName}: unavailable in read-only mode.`)
  }
  const action = typeof args.action === 'string' ? args.action : 'list'
  return action === 'list' || action === 'active'
    ? null
    : errorResult(
        'tabs: read-only mode only supports action="list" or "active".',
      )
}
