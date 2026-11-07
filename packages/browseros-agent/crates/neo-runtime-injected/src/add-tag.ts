/**
 * Playwright: `server/frames.ts` `Frame._addScriptTag`
 * Playwright: `server/frames.ts` `Frame._addStyleTag`
 *
 * The bundle entry `dist/add-tag.js`, run as `UtilityScript.evaluate`'s expression in the page's
 * own world: Playwright's four page functions behind `addScriptTag` and `addStyleTag`, chosen by
 * name, answering the new element.
 *
 * Deviation: `server/frames.ts` `Frame._raceWithCSPError`: `if (message.text().includes('Content-Security-Policy') || message.text().includes('Content Security Policy')) {`
 * Playwright rejects a tag the page's Content Security Policy blocked by watching the console;
 * Neo watches no console, so it waits for the document's `securitypolicyviolation` event
 * instead, one task after the tag (Chromium reports an inline script's violation in a later
 * task), and rejects with an Error naming the policy.
 */

import type { UtilityScript } from '../vendor/playwright/injected/src/utilityScript'

type Tag = (value: string, type: string) => HTMLElement | Promise<HTMLElement>

const TAGS: Record<
  'scriptUrl' | 'scriptContent' | 'styleUrl' | 'styleContent',
  Tag
> = {
  // Playwright: `server/frames.ts` `Frame._addScriptTag/addScriptUrl`: `script.src = params.url;`
  scriptUrl: async (url, type) => {
    const script = document.createElement('script')
    script.src = url
    if (type) script.type = type
    const promise = new Promise((resolve, reject) => {
      script.onload = resolve
      script.onerror = (e) =>
        reject(
          typeof e === 'string'
            ? new Error(e)
            : new Error(`Failed to load script at ${script.src}`),
        )
    })
    document.head.appendChild(script)
    await promise
    return script
  },
  // Playwright: `server/frames.ts` `Frame._addScriptTag/addScriptContent`: `script.text = params.content;`
  scriptContent: (content, type) => {
    const script = document.createElement('script')
    script.type = type || 'text/javascript'
    script.text = content
    let error: unknown = null
    script.onerror = (e) => {
      error = e
    }
    document.head.appendChild(script)
    if (error) throw error
    return script
  },
  // Playwright: `server/frames.ts` `Frame._addStyleTag/addStyleUrl`: `link.href = url;`
  styleUrl: async (url) => {
    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.href = url
    const promise = new Promise((resolve, reject) => {
      link.onload = resolve
      link.onerror = reject
    })
    document.head.appendChild(link)
    await promise
    return link
  },
  // Playwright: `server/frames.ts` `Frame._addStyleTag/addStyleContent`: `style.appendChild(document.createTextNode(content));`
  styleContent: async (content) => {
    const style = document.createElement('style')
    style.type = 'text/css'
    style.appendChild(document.createTextNode(content))
    const promise = new Promise((resolve, reject) => {
      style.onload = resolve
      style.onerror = reject
    })
    document.head.appendChild(style)
    await promise
    return style
  },
}

/** Neo: Playwright's CSP error text, rebuilt from the violation the document reported. */
function cspError(event: SecurityPolicyViolationEvent): Error {
  return new Error(
    `Refused to load or run ${event.blockedURI || 'inline content'} because it violates the following Content Security Policy directive: "${event.violatedDirective}".`,
  )
}

/** Neo: the tag `kind` with `value` (a URL or the content), raced against a CSP violation as
 * Playwright races it against the CSP console error: a blocked tag may never load or fail. */
export default async function addTag(
  utilityScript: UtilityScript,
  kind: keyof typeof TAGS,
  value: string,
  type: string,
): Promise<HTMLElement> {
  let record: (event: SecurityPolicyViolationEvent) => void = () => {}
  const violation = new Promise<never>((_resolve, reject) => {
    record = (event) => reject(cspError(event))
  })
  document.addEventListener('securitypolicyviolation', record)
  try {
    const element = await Promise.race([TAGS[kind](value, type), violation])
    // Chromium reports an inline script's violation in a later task (Playwright's
    // `cspErrorsAsynchronousForInlineScripts`).
    await Promise.race([
      new Promise((resolve) => utilityScript.builtins.setTimeout(resolve, 0)),
      violation,
    ])
    return element
  } finally {
    document.removeEventListener('securitypolicyviolation', record)
  }
}
