/** Neo: Verifies declared bootstrap conditions and tab activation without acquiring a browser. */
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import {
  bootstrapInitialPage,
  contextBootstrapPlan,
  OwnedBrowser,
  projectActivation,
} from './browser.mjs'

test('upstream defaults explicitly establish the original 1280 by 720 context conditions', async () => {
  const plan = contextBootstrapPlan()
  assert.deepEqual(plan.contextOptions.viewport, { width: 1280, height: 720 })
  assert.deepEqual(plan.fixtureOptions, {
    actionTimeout: 0,
    navigationTimeout: 0,
    testIdAttribute: 'data-testid',
  })
  assert.deepEqual(plan.limitations, [])
  assert.deepEqual(contextBootstrapPlan(plan.contextOptions).limitations, [])
  assert.deepEqual(
    contextBootstrapPlan({ viewport: undefined }).contextOptions.viewport,
    { width: 1280, height: 720 },
  )
  const calls = []
  const cdp = {
    async send(method, params, session) {
      calls.push({ method, params, session })
      if (method === 'Target.getTargetInfo')
        return { targetInfo: { type: 'page', url: 'about:blank' } }
      return {}
    },
  }
  await bootstrapInitialPage(
    cdp,
    'owned',
    plan,
    'observer-session',
    '/owned/downloads',
  )
  assert.deepEqual(
    calls.map((call) => call.method),
    [
      'Target.getTargetInfo',
      'Emulation.setDeviceMetricsOverride',
      'Emulation.setEmulatedMedia',
      'Emulation.setLocaleOverride',
      'Browser.setDownloadBehavior',
    ],
  )
  assert.deepEqual(calls[1].params, {
    width: 1280,
    height: 720,
    screenWidth: 1280,
    screenHeight: 720,
    deviceScaleFactor: 1,
    mobile: false,
    screenOrientation: { angle: 0, type: 'landscapePrimary' },
  })
  assert.ok(
    calls.slice(1, 4).every((call) => call.session === 'observer-session'),
  )
  assert.equal(calls[3].params.locale, 'en-US')
  assert.deepEqual(calls[4].params, {
    behavior: 'allowAndName',
    downloadPath: '/owned/downloads',
    eventsEnabled: true,
  })
})

test('nondefault upstream options become explicit limitations before any bootstrap command', async () => {
  for (const [key, value] of Object.entries({
    viewport: { width: 500, height: 400 },
    ignoreHTTPSErrors: true,
    locale: 'de-DE',
    storageState: { cookies: [], origins: [] },
    screen: { width: 1024, height: 768 },
  })) {
    const plan = contextBootstrapPlan({ [key]: value })
    assert.match(plan.limitations.join(), new RegExp(key))
    assert.deepEqual(plan.contextOptions[key], value)
    await assert.rejects(
      bootstrapInitialPage(
        {
          send() {
            assert.fail('unsupported setup issued CDP')
          },
        },
        'owned',
        plan,
        'session',
      ),
      new RegExp(key),
    )
  }
  for (const [key, value] of Object.entries({
    actionTimeout: 50,
    navigationTimeout: 200,
    testIdAttribute: 'data-other',
  }))
    assert.match(
      contextBootstrapPlan({}, { [key]: value }).limitations.join(),
      new RegExp(key),
    )
})

test('bootstrap cannot configure a navigated page or operate without an owned session', async () => {
  await assert.rejects(
    bootstrapInitialPage(
      {
        async send() {
          return { targetInfo: { type: 'page', url: 'https://test.invalid/' } }
        },
      },
      'owned',
      contextBootstrapPlan(),
      'session',
    ),
    /initial blank page/,
  )
  await assert.rejects(
    bootstrapInitialPage(
      {
        async send() {
          return { targetInfo: { type: 'page', url: 'about:blank' } }
        },
      },
      'owned',
      contextBootstrapPlan(),
    ),
    /owned observer session/,
  )
})

test('activation compares tab-strip state independently of document focus and raw IDs', () => {
  const pages = [
    { targetId: 'a', ordinal: 0 },
    { targetId: 'closed', ordinal: 1, closed: true },
    { targetId: 'b', ordinal: 2 },
  ]
  const tabs = [
    { targetId: 'a', windowId: 90, index: 0, isActive: true },
    { targetId: 'b', windowId: 90, index: 1, isActive: false },
  ]
  const before = projectActivation(tabs, pages)
  assert.deepEqual(before, {
    status: 'complete',
    tabs: [
      { page: 0, window: 0, index: 0, active: true },
      { page: 2, window: 0, index: 1, active: false },
    ],
  })
  assert.deepEqual(
    projectActivation(
      tabs.map((tab) => ({ ...tab, windowId: 130 })),
      pages,
    ),
    before,
  )
  assert.notDeepEqual(
    projectActivation(
      tabs.map((tab) => ({ ...tab, isActive: !tab.isActive })),
      pages,
    ),
    before,
  )
  for (const invalid of [
    undefined,
    [],
    [...tabs, tabs[0]],
    tabs.map((tab) => ({ ...tab, isActive: false })),
    tabs.map(({ index, ...tab }) => tab),
  ])
    assert.throws(() => projectActivation(invalid, pages))
  assert.deepEqual(projectActivation([], []), { status: 'complete', tabs: [] })
})

test('pre-aborted browser startup acquires no profile or browser process', async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'neo-bootstrap-abort-'))
  t.after(() => fs.rm(root, { recursive: true, force: true }))
  const outputDir = path.join(root, 'never-acquired')
  const signal = AbortSignal.abort(new Error('cancelled before startup'))
  await assert.rejects(
    OwnedBrowser.start({
      executable: '/no-browser-must-launch',
      outputDir,
      signal,
    }),
    /cancelled before startup/,
  )
  assert.deepEqual(await fs.readdir(root), [])
})

test('forced helper cleanup reports final survivors separately from processes it killed', {
  timeout: 15_000,
}, async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'neo-bootstrap-helper-'))
  t.after(() => fs.rm(root, { recursive: true, force: true }))
  const executable = path.join(root, 'fake-browser')
  const helperFile = path.join(root, 'helper.pid')
  const helper = `process.on('SIGTERM', () => {});
    require('node:fs').writeFileSync(process.argv[1], String(process.pid));
    setInterval(() => {}, 1000);`
  await fs.writeFile(
    executable,
    `#!/usr/bin/env node
    const fs = require('node:fs');
    const {spawn} = require('node:child_process');
    spawn(process.execPath, ['-e', ${JSON.stringify(helper)}, ${JSON.stringify(helperFile)}], {stdio:'ignore'});
    const timer = setInterval(() => {
      if (fs.existsSync(${JSON.stringify(helperFile)})) { clearInterval(timer); process.exit(0); }
    }, 10);
  `,
    { mode: 0o700 },
  )
  const outputDir = path.join(root, 'output')
  await assert.rejects(
    OwnedBrowser.start({ executable, outputDir }),
    /Browser exited before publishing/,
  )
  const cleanup = JSON.parse(
    await fs.readFile(path.join(outputDir, 'cleanup.json'), 'utf8'),
  )
  const helperPid = Number(await fs.readFile(helperFile, 'utf8'))
  assert.equal(cleanup.status, 'complete')
  assert.equal(cleanup.forced, true)
  assert(cleanup.forcedProcesses.some((process) => process.pid === helperPid))
  assert.deepEqual(cleanup.survivors, [])
  assert.throws(
    () => process.kill(helperPid, 0),
    (error) => error.code === 'ESRCH',
  )
})

test('abort during endpoint discovery reaps an already started owned child before rejecting', {
  timeout: 10_000,
}, async (t) => {
  const root = await fs.mkdtemp(
    path.join(os.tmpdir(), 'neo-bootstrap-starting-'),
  )
  t.after(() => fs.rm(root, { recursive: true, force: true }))
  const executable = path.join(root, 'fake-browser')
  await fs.writeFile(
    executable,
    `#!/bin/sh
for argument in "$@"; do
  case "$argument" in
    --user-data-dir=*) profile="\${argument#--user-data-dir=}" ;;
  esac
done
printf '%s\\n' "$$" > "$profile/startup-marker"
exec sleep 60
`,
    { mode: 0o700 },
  )
  const outputDir = path.join(root, 'output')
  const controller = new AbortController()
  const pending = OwnedBrowser.start({
    executable,
    outputDir,
    signal: controller.signal,
  })
  const rejected = assert.rejects(pending, /cancelled during startup/)
  try {
    const deadline = Date.now() + 3_000
    let started = false
    while (!started && Date.now() < deadline) {
      const entries = await fs.readdir(outputDir).catch(() => [])
      for (const entry of entries.filter((name) => name.startsWith('profile-')))
        started =
          started ||
          (await fs.stat(path.join(outputDir, entry, 'startup-marker')).then(
            () => true,
            () => false,
          ))
      if (!started) await new Promise((resolve) => setTimeout(resolve, 10))
    }
    assert.equal(
      started,
      true,
      'fake browser acquired its owned process before cancellation',
    )
    controller.abort(new Error('cancelled during startup'))
    await rejected
    const cleanup = JSON.parse(
      await fs.readFile(path.join(outputDir, 'cleanup.json'), 'utf8'),
    )
    assert.equal(cleanup.status, 'complete')
    assert.deepEqual(cleanup.survivors, [])
    assert.ok(
      !(await fs.readdir(outputDir)).some((entry) =>
        entry.startsWith('profile-'),
      ),
    )
  } finally {
    controller.abort(new Error('cancelled during startup'))
    await rejected
  }
})
