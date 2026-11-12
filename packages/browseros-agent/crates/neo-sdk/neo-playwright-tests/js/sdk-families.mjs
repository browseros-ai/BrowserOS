/** Neo: Reflect the SDK's public classes without constructing objects or calling getters. */
import fs from 'node:fs'
import vm from 'node:vm'

// Explicit public names exclude helpers such as Connection, EventEmitter and CSS tokens.
const families = [
  'APIRequest',
  'APIResponse',
  'Browser',
  'BrowserContext',
  'BrowserType',
  'Clock',
  'ConsoleMessage',
  'Debugger',
  'Dialog',
  'Disposable',
  'DisposableStub',
  'Download',
  'ElementHandle',
  'FileChooser',
  'Frame',
  'FrameLocator',
  'JSHandle',
  'Keyboard',
  'Locator',
  'Mouse',
  'Page',
  'Playwright',
  'Request',
  'Response',
  'Route',
  'Selectors',
  'Touchscreen',
  'WebError',
  'WebStorage',
]

let cached
export function sdkCapabilities() {
  if (cached) return cached
  const frozen = new URL('./sdk-capabilities.json', import.meta.url)
  if (fs.existsSync(frozen)) {
    cached = JSON.parse(fs.readFileSync(frozen, 'utf8'))
    return cached
  }
  const bundle = fs.readFileSync(
    new URL('../../neo-quickjs-injected/dist/sdk.js', import.meta.url),
    'utf8',
  )
  const exports = 'return { createRun };'
  if (bundle.split(exports).length !== 2)
    throw new Error(
      'SDK bundle export shape changed; cannot inventory public classes',
    )
  // The committed, unminified bundle keeps these lexical class names. A private VM
  // exposes only their prototypes; createRun and the native bridge are never called.
  const classes = vm.runInNewContext(
    bundle.replace(exports, `return { ${families.join(', ')} };`),
  )
  cached = Object.fromEntries(
    families.map((family) => {
      const members = new Set()
      for (
        let prototype = classes[family].prototype;
        prototype && Object.getPrototypeOf(prototype);
        prototype = Object.getPrototypeOf(prototype)
      )
        for (const member of Object.getOwnPropertyNames(prototype))
          if (member !== 'constructor') members.add(member)
      return [family, [...members].sort()]
    }),
  )
  return cached
}
