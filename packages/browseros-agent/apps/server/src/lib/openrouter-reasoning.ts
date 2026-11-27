/** OpenRouter's Flash route requires reasoning and defaults to max effort.
 * https://openrouter.ai/api/v1/models
 * Other routes keep the existing gateway-selected reasoning behavior.
 */
export function getOpenRouterReasoning(
  model: string,
  effort?: string | null,
): { effort?: 'low' | 'medium' | 'high' | 'xhigh' | 'max' } {
  if (model !== 'minimax/minimax-m3.1-flash:free') return {}
  switch (effort) {
    case undefined:
    case null:
      return { effort: 'max' }
    case 'low':
    case 'medium':
    case 'high':
    case 'xhigh':
    case 'max':
      return { effort }
    default:
      throw new Error(
        'MiniMax M3.1 Flash requires low, medium, high, xhigh, or max reasoning effort',
      )
  }
}
