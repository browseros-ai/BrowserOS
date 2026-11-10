/** Neo: Keeps caller selection inside the pinned corpus and the four-browser ownership limit. */

export function runnerOptions(argv) {
  const options = {
    reference: false,
    skipBuild: false,
    debugBuild: false,
    expectLoop: true,
    workers: 4,
    args: [],
  }
  const inversions = []
  let files = []
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]
    if (arg === '--') {
      files = argv.slice(index + 1)
      break
    }
    if (arg === '--reference') options.reference = true
    else if (arg === '--skip-build') options.skipBuild = true
    else if (arg === '--debug-build') options.debugBuild = true
    else if (arg === '--no-expect-loop') options.expectLoop = false
    else if (arg === '--release') options.debugBuild = false
    else if (/^--workers(=|$)|^-j/.test(arg)) {
      const attached = arg.startsWith('-j')
        ? arg.slice(2).replace(/^=/, '')
        : arg.slice('--workers'.length).replace(/^=/, '')
      const value = attached || argv[++index]
      if (!/^[1-4]$/.test(value || ''))
        throw new Error('--workers must be between 1 and 4')
      options.workers = Number(value)
    } else if (/^--grep-invert(=|$)|^-G/.test(arg)) {
      const attached = arg.startsWith('-G')
        ? arg.slice(2).replace(/^=/, '')
        : arg.slice('--grep-invert'.length).replace(/^=/, '')
      const value = attached || argv[++index]
      if (!value) throw new Error('--grep-invert requires a pattern')
      inversions.push(value)
    } else if (
      /^(--headed|--debug|--ui|--ui-host|--ui-port|--config|--reporter|--output|--browser|--update-snapshots)(=|$)|^-[cu]/.test(
        arg,
      )
    ) {
      throw new Error(
        `${arg} would override the headless corpus/output contract`,
      )
    } else options.args.push(arg)
  }
  // A CLI grep replaces configuration grep. Always compose this restriction at the CLI seam.
  inversions.push('\\bheadful\\.spec\\.ts\\b(?!.*\\bHE-012\\b)')
  options.args.push(
    '--grep-invert',
    inversions.map((value) => `(?:${value})`).join('|'),
  )
  if (files.length) options.args.push('--', ...files)
  if (options.reference) options.expectLoop = false
  return options
}
