/**
 * Whether the current extension page runs in an incognito context. Assistant
 * history is never persisted there (#1189). `chrome.extension.inIncognitoContext`
 * is synchronous and reads true for an extension page (side panel, newtab)
 * opened in an incognito window, so the gate can be applied without racing an
 * async lookup.
 */
export function isIncognitoContext(): boolean {
  return chrome.extension?.inIncognitoContext ?? false
}
