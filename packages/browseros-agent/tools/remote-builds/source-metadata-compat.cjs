/** Supply the original Mac HEAD to legacy spec reports without uploading repository history.
 * Only the exact read of HEAD in the uploaded agent directory is adapted. Public Playwright
 * Git commands and every other child process continue through Node's original implementation.
 */
/* biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Installed remote-job preload, not a cached Turbo task. */
if (!process.versions.bun) {
  const child = require('node:child_process')
  const fs = require('node:fs')
  const path = require('node:path')
  const { syncBuiltinESMExports } = require('node:module')
  const revision = process.env.NEO_SDK_SOURCE_REVISION
  const root = process.env.NEO_SDK_SOURCE_ROOT
  const dirty = process.env.NEO_SDK_SOURCE_DIRTY
  if (
    !root ||
    !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(revision || '') ||
    !['true', 'false'].includes(dirty)
  )
    throw new Error(
      'Remote spec metadata requires root, full revision and dirty flag',
    )
  const agent = fs.realpathSync(path.join(root, 'packages/browseros-agent'))
  const original = child.execFileSync
  child.execFileSync = (...call) => {
    const [file, args, options = {}] = call
    if (
      file === 'git' &&
      args?.length === 2 &&
      args[0] === 'rev-parse' &&
      args[1] === 'HEAD' &&
      fs.realpathSync(options.cwd || process.cwd()) === agent
    ) {
      const value = Buffer.from(`${revision}\n`)
      return options.encoding && options.encoding !== 'buffer'
        ? value.toString(options.encoding)
        : value
    }
    return original(...call)
  }
  syncBuiltinESMExports()
}
