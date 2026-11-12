/** Neo: Keeps caller selection inside the pinned corpus and the browser ownership limit. */

/**
 * Of headful.spec.ts only HE-012 (`Page.bringToFront should work`) runs, headless by owner
 * policy; every other case in that file needs a headed browser. The pinned titles carry no IDs,
 * so the exception is matched by title.
 */
export const headfulInvert =
  '\\bheadful\\.spec\\.ts\\b(?!.*\\bPage\\.bringToFront should work\\b)'

/**
 * Each worker owns one headless BrowserOS (browser-owner.mjs holds its slot until that browser is
 * gone). Four is the default because developer machines run several builds at once; the CI job
 * (neo-spec.yml) has a 32-vCPU runner to itself and passes more. The cap keeps a typo from
 * starting a browser per test file.
 */
export const MAX_WORKERS = 32

/**
 * Where Neo opens each test's page (the add-on's `ownWindowPages`). On Linux a background tab is
 * never drawn, and Chromium's compositor then sends it one frame a second
 * (`ThrottleUndrawnFrames` in viz's `CompositorFrameSinkSupport::ShouldSendBeginFrame`): every
 * mouse move waits ~1 s and screenshots stall (SS-054, LH-001). Playwright's own pages are each
 * the active tab of their window, so on Linux the harness opens Neo's pages in a window of their
 * own too, and the corpus measures Neo rather than that throttle. macOS draws background tabs, so
 * it keeps the product's placement. `--background-tabs` and `--own-window-pages` choose either
 * one on any platform.
 */
export const defaultOwnWindowPages = (platform = process.platform) =>
  platform === 'linux'

export function runnerOptions(argv) {
  const options = {
    engine: 'claw',
    hostedMode: false,
    ownWindowPages: defaultOwnWindowPages(),
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
    else if (arg === '--own-window-pages') options.ownWindowPages = true
    else if (arg === '--background-tabs') options.ownWindowPages = false
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
      const workers = /^\d+$/.test(value || '') ? Number(value) : 0
      if (workers < 1 || workers > MAX_WORKERS)
        throw new Error(`--workers must be between 1 and ${MAX_WORKERS}`)
      options.workers = workers
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
