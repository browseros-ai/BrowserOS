/**
 * Whether the assistant is running in an incognito window. Assistant history is
 * never persisted when true (#1189).
 *
 * Resolved from the hosting window rather than `chrome.extension.inIncognitoContext`:
 * in the default "spanning" incognito mode the side panel runs in the shared
 * (non-incognito) context, so `inIncognitoContext` reads false even when the
 * panel is shown in an incognito window. The window's own `incognito` flag is
 * authoritative. `chrome.windows` needs no permission and `tabs` is already
 * granted, so no manifest change is required.
 */
export async function isIncognitoWindow(): Promise<boolean> {
  try {
    const window = await chrome.windows.getCurrent()
    return window.incognito ?? false
  } catch {
    return false
  }
}
