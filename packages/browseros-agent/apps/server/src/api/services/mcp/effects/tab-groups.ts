import type { BrowserToolEffect } from '../browser-tool-dispatch'

/** Queues cosmetic grouping only for tabs the agent created successfully. */
export const applyTabGroups: BrowserToolEffect = ({ call, result }) => {
  if (
    result.isError ||
    !call.run?.panelsVisible ||
    !call.run.tabGroup ||
    call.trace.created.size === 0
  ) {
    return
  }
  // Group synchronization is deliberately detached: Chrome grouping must not
  // delay or fail the browser operation whose result the model is awaiting.
  call.tabGroups.addCreatedPages(call.run, [...call.trace.created])
}
