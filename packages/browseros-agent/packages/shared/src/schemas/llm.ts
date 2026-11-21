/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export const CONVERSATION_ID_PLACEHOLDER = '{{conversationId}}'
// Unlike $, the final assertion rejects trailing newlines too.
export const HEADER_NAME_PATTERN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\s\S])/
export const HEADER_VALUE_PATTERN = /^[\t\x20-\x7e\x80-\xff]*(?![\s\S])/
