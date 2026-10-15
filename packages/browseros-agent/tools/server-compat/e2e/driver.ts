/** Uses test-ui's CDP inspector on an owned profile, then reports explicit manual evidence for each release journey. */
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { bootServer, parseKinds, runCli, workspace } from '../boot'
import { checklist } from './checklist'

interface Receipt {
  status: 'PASS' | 'FAIL' | 'SKIP'
  reason: string
  uiEvidence?: string
  serverEvidence?: string
}
interface Evidence {
  runtime: 'ts' | 'rust'
  platform: string
  steps: Record<string, Receipt>
}
const argument = (flag: string) => {
  const index = process.argv.indexOf(flag)
  return index < 0 ? undefined : process.argv[index + 1]
}

async function inspect(port: number, args: string[], output: string) {
  const child = Bun.spawn(
    [process.execPath, 'scripts/dev/inspect-ui.ts', ...args],
    {
      cwd: workspace,
      env: { ...process.env, BROWSEROS_CDP_PORT: String(port) },
      stdout: 'pipe',
      stderr: 'pipe',
    },
  )
  const timer = setTimeout(() => child.kill('SIGKILL'), 20_000)
  try {
    const [stdout, stderr, code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    await writeFile(output, stdout + stderr)
    if (code !== 0) throw new Error(`CDP ${args[0]} failed: ${stdout}${stderr}`)
    return stdout
  } finally {
    clearTimeout(timer)
  }
}

async function main() {
  const kind = parseKinds().a
  const output = resolve(
    process.env.SERVER_COMPAT_OUTPUT ??
      (await mkdtemp(join(tmpdir(), 'server-compat-e2e-'))),
  )
  await mkdir(output, { recursive: true })
  console.log(`Artifacts: ${output}`)
  const template = argument('--write-template')
  if (template) {
    await writeFile(
      resolve(template),
      JSON.stringify(
        {
          runtime: kind,
          platform: process.platform,
          steps: Object.fromEntries(
            checklist.map(([id, reason]) => [
              id,
              { status: 'SKIP', reason, uiEvidence: '', serverEvidence: '' },
            ]),
          ),
        },
        null,
        2,
      ),
    )
    console.log(`Evidence template: ${resolve(template)}`)
  }
  const evidenceFile = argument('--evidence')
  let evidence: Evidence | undefined
  if (evidenceFile) {
    evidence = JSON.parse(
      await readFile(resolve(evidenceFile), 'utf8'),
    ) as Evidence
    if (
      evidence.runtime !== kind ||
      evidence.platform !== process.platform ||
      !evidence.steps
    )
      throw new Error('Evidence runtime/platform must match this run')
    const unknown = Object.keys(evidence.steps).filter(
      (key) => !checklist.some(([id]) => key === id),
    )
    if (unknown.length)
      throw new Error(`Unknown checklist ids: ${unknown.join(', ')}`)
  }
  const extensionDir = argument('--extension')
  const receipts: Array<{ id: string } & Receipt> = []
  if (extensionDir) {
    if (!existsSync(join(resolve(extensionDir), 'manifest.json')))
      throw new Error('Extension directory must contain a built manifest.json')
    const server = await bootServer(kind, { extensionDir, headless: false })
    try {
      const targets = (await fetch(
        `http://127.0.0.1:${server.cdpPort}/json/list`,
        { signal: AbortSignal.timeout(5_000) },
      ).then((response) => response.json())) as Array<{
        type: string
        url: string
      }>
      await inspect(server.cdpPort, ['targets'], join(output, 'targets.txt'))
      console.log(
        `PASS cdp-targets: inspector connected to this run's temporary profile (CDP ${server.cdpPort})`,
      )
      // Snapshot ids belong to one render only. This driver does not reuse them or touch owner tabs.
      for (const surface of ['app.html', 'sidepanel']) {
        if (
          !targets.some(
            (target) =>
              target.type === 'page' &&
              target.url.startsWith('chrome-extension://') &&
              target.url.includes(surface),
          )
        ) {
          console.log(
            `SKIP ${surface}-snapshot: extension surface not open; use the test-ui open-sidepanel/navigation flow in an isolated dev session`,
          )
          continue
        }
        await inspect(
          server.cdpPort,
          ['snapshot', surface],
          join(output, `${surface}.snapshot.txt`),
        )
        await inspect(
          server.cdpPort,
          ['screenshot', surface, join(output, `${surface}.png`)],
          join(output, `${surface}.screenshot.txt`),
        )
        console.log(
          `PASS ${surface}-snapshot: accessibility snapshot and screenshot saved`,
        )
      }
    } finally {
      await server.stop()
    }
  } else
    console.log(
      'SKIP cdp-targets: supply --extension <built chrome-mv3 directory> to launch an owned BrowserOS profile',
    )
  for (const [id, instructions] of checklist) {
    const receipt = evidence?.steps[id] ?? {
      status: 'SKIP' as const,
      reason: `Not executed: ${instructions}`,
    }
    if (
      !['PASS', 'FAIL', 'SKIP'].includes(receipt.status) ||
      typeof receipt.reason !== 'string' ||
      !receipt.reason.trim()
    )
      throw new Error(`Invalid receipt for ${id}`)
    if (receipt.status === 'PASS') {
      // A PASS is a human attestation with both observable sides attached, never just a checked box.
      for (const field of ['uiEvidence', 'serverEvidence'] as const) {
        const file = receipt[field]
        const path =
          file &&
          (isAbsolute(file)
            ? file
            : resolve(dirname(resolve(evidenceFile!)), file))
        if (!path || !existsSync(path))
          throw new Error(`${id} PASS requires an existing ${field} artifact`)
      }
    }
    receipts.push({ id, ...receipt })
    console.log(
      `${receipt.status} ${id}: ${receipt.reason}${receipt.status === 'PASS' ? ' (manual evidence)' : ''}`,
    )
  }
  await writeFile(
    join(output, 'checklist.json'),
    JSON.stringify(
      { runtime: kind, platform: process.platform, receipts },
      null,
      2,
    ),
  )
  if (receipts.some((receipt) => receipt.status === 'FAIL'))
    throw new Error('Extension checklist contains failures')
  if (
    process.argv.includes('--required') &&
    (!extensionDir || receipts.some((receipt) => receipt.status !== 'PASS'))
  )
    throw new Error(
      'Required extension acceptance is incomplete; SKIP never counts as PASS',
    )
}
if (import.meta.main) await runCli(main)
