/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  buildSessionGroupTitle,
  clientPrefixFromSlug,
  identityService,
} from '../../lib/mcp-session'
import type { ToolEffect } from '../dispatch'

/** Appends a bounded rename nudge while the session keeps its generated label. */
export const applySessionNaming: ToolEffect = ({ call, result }) => {
  const identity = call.identity
  if (result.isError || call.tool.name === 'name_session' || !identity) {
    return undefined
  }
  if (!identityService.takeRenameNudge(call.sessionId)) return undefined

  const title = buildSessionGroupTitle(
    clientPrefixFromSlug(identity.slug),
    identity.label,
  )
  const tip = `Tip: this session is "${title}" — rename it with name_session name="<2-3 word task label>"`
  return {
    ...result,
    content: [...result.content, { type: 'text', text: tip }],
  }
}
