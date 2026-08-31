/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export class ServerActivity {
  private activeChatStreams = 0
  private activeMcpToolExecutions = 0

  beginChatStream(): void {
    this.activeChatStreams += 1
  }

  endChatStream(): void {
    this.activeChatStreams = Math.max(0, this.activeChatStreams - 1)
  }

  beginMcpToolExecution(): void {
    this.activeMcpToolExecutions += 1
  }

  endMcpToolExecution(): void {
    this.activeMcpToolExecutions = Math.max(0, this.activeMcpToolExecutions - 1)
  }

  isBusy(): boolean {
    return this.activeChatStreams > 0 || this.activeMcpToolExecutions > 0
  }
}
