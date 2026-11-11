/** Neo: Keeps caller selection inside the pinned corpus and the four-browser ownership limit. */

/**
 * Of headful.spec.ts only HE-012 (`Page.bringToFront should work`) runs, headless by owner
 * policy; every other case in that file needs a headed browser. The pinned titles carry no IDs,
 * so the exception is matched by title.
 */
export const headfulInvert =
  '\\bheadful\\.spec\\.ts\\b(?!.*\\bPage\\.bringToFront should work\\b)'

export function runnerOptions(argv) {
  const options = {
    engine: 'claw',
    hostedMode: false,
    reference: false,
    skipBuild: false,
    debugBuild: false,
    expectLoop: true,
    curation: true,
    workers: 4,
    args: [],
  }
  let selectedEngine
  const selectEngine = (engine) => {
    if (!['reference', 'claw', 'neo'].includes(engine))
      throw new Error('--engine must be reference, claw or neo')
    if (selectedEngine && selectedEngine !== engine)
      throw new Error('Conflicting engine switches')
    selectedEngine = engine
    options.engine = engine
  }
  const inversions = []
  let files = []
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]
    if (arg === '--') {
      files = argv.slice(index + 1)
      if (files.some((file) => file.startsWith('-')))
        throw new Error('Test filters after -- cannot start with a dash')
      break
    }
    if (arg === '--reference') selectEngine('reference')
    else if (arg === '--engine') selectEngine(argv[++index])
    else if (arg.startsWith('--engine=')) selectEngine(arg.slice(9))
    else if (arg === '--hosted-mode') options.hostedMode = true
    else if (arg === '--no-curation') options.curation = false
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
  inversions.push(headfulInvert)
  options.args.push(
    '--grep-invert',
    inversions.map((value) => `(?:${value})`).join('|'),
  )
  // The pinned Playwright treats its own -- tail as user data, not test filters.
  options.args.push(...files)
  options.reference = options.engine === 'reference'
  if (options.reference) options.expectLoop = false
  return options
}
