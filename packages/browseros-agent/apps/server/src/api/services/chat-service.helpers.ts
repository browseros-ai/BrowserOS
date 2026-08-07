/**
 * Pure builders for the model-visible notices that describe mid-conversation
 * changes to a cached agent session's inputs (connected MCP servers, workspace,
 * chat mode). Kept side-effect free so ChatService.processMessage stays plain
 * control flow: detect the change, rebuild once, then emit these notices.
 */

/** Notice for a change in the connected MCP / app-integration set. */
export function describeMcpChange(
  previousMcpKey: string | undefined,
  currentMcpKey: string,
): string {
  const oldParts = (previousMcpKey ?? '').split(',').filter(Boolean)
  const newParts = currentMcpKey.split(',').filter(Boolean)
  const oldKlavisState = oldParts.find((s) => s.startsWith('klavis:'))
  const newKlavisState = newParts.find((s) => s.startsWith('klavis:'))
  const oldServers = new Set(oldParts.filter((s) => !s.startsWith('klavis:')))
  const newServers = new Set(newParts.filter((s) => !s.startsWith('klavis:')))
  const added = [...newServers].filter((s) => !oldServers.has(s))
  const removed = [...oldServers].filter((s) => !newServers.has(s))

  const parts: string[] = []
  if (removed.length > 0) {
    parts.push(
      `The following app integrations were disconnected: ${removed.join(', ')}. Their tools are no longer available.`,
    )
  }
  if (added.length > 0) {
    parts.push(
      `The following app integrations were connected: ${added.join(', ')}. Their tools are now available.`,
    )
  }
  if (parts.length === 0) {
    if (
      oldKlavisState !== 'klavis:ready' &&
      newKlavisState === 'klavis:ready' &&
      newServers.size > 0
    ) {
      parts.push(
        `Klavis app integration tools are now available for the following connected apps: ${[...newServers].join(', ')}.`,
      )
    } else {
      parts.push(
        'Connected app integrations changed during this conversation. Use only tools that are currently registered.',
      )
    }
  }
  return parts.join(' ')
}

/** Notice for a workspace connect, disconnect, or switch mid-conversation. */
export function describeWorkspaceChange(
  previousWorkingDir: string | undefined,
  currentWorkingDir: string | undefined,
  chatMode: boolean,
): string {
  if (!currentWorkingDir) {
    return [
      'The user disconnected the workspace during this conversation.',
      'Workspace filesystem tools (filesystem_write, filesystem_edit, filesystem_bash, filesystem_grep, filesystem_find, filesystem_ls, and workspace file reads) are no longer available.',
      'filesystem_read can only read BrowserOS-generated output files returned in this session.',
      'Return other output directly in chat.',
      'If the user asks for file operations, suggest they select a working directory from the chat toolbar.',
    ].join(' ')
  }
  if (!previousWorkingDir) {
    return chatMode
      ? [
          'The user connected a workspace during this conversation, but read-only chat mode cannot use workspace filesystem tools.',
          'filesystem_read can only read BrowserOS-generated output files returned in this session.',
        ].join(' ')
      : `The user connected a workspace during this conversation. Filesystem tools are now available. Working directory: ${currentWorkingDir}`
  }
  return chatMode
    ? [
        'The user switched workspace during this conversation, but read-only chat mode cannot use workspace filesystem tools.',
        'filesystem_read can only read BrowserOS-generated output files returned in this session.',
      ].join(' ')
    : `The user switched workspace during this conversation. Filesystem tools now use the new working directory: ${currentWorkingDir}`
}

/** Notice for a chat/agent mode switch mid-conversation. */
export function describeModeChange(
  chatMode: boolean,
  workspaceConnected: boolean,
): string {
  if (chatMode) {
    return workspaceConnected
      ? 'The user switched to read-only chat mode during this conversation. You can observe pages but can no longer interact with them, and workspace filesystem tools other than reading BrowserOS output files are no longer available.'
      : 'The user switched to read-only chat mode during this conversation. You can observe pages but can no longer interact with them or modify files.'
  }
  return workspaceConnected
    ? 'The user switched to agent mode during this conversation. You can interact with pages and perform browser actions again, and the workspace filesystem tools are available again. Any earlier statement that read-only chat mode blocked them no longer applies.'
    : 'The user switched to agent mode during this conversation. You can interact with pages and perform browser actions again.'
}
