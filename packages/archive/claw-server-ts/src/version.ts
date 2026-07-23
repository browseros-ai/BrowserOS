import pkg from '../package.json' with { type: 'json' }

declare const __BROWSEROS_VERSION__: string

export const VERSION: string =
  typeof __BROWSEROS_VERSION__ !== 'undefined'
    ? __BROWSEROS_VERSION__
    : pkg.version
