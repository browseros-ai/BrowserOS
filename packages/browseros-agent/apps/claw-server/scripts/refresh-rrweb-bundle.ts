#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pulls a specific rrweb release tarball from npm and copies the
 * UMD minified bundle into src/vendor/. Run when we deliberately
 * decide to bump rrweb; do NOT run on every install. The bundle is
 * committed to source control so the recorder script that runs
 * inside agent pages is byte-for-byte reproducible.
 *
 * Honours the project's bunfig minimum-release-age policy: refuses
 * to fetch a version younger than seven days. Override with
 * --allow-young if intentional.
 *
 * Usage:
 *   bun run apps/claw-server/scripts/refresh-rrweb-bundle.ts --version 2.0.1
 *   bun run apps/claw-server/scripts/refresh-rrweb-bundle.ts --version 2.0.1 --allow-young
 */
// biome-ignore-all lint/suspicious/noConsole: this is a CLI script

import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { $ } from 'bun'

interface Args {
  version: string
  allowYoung: boolean
}

function parseArgs(argv: string[]): Args {
  let version: string | null = null
  let allowYoung = false
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--version') version = argv[++i] ?? null
    else if (a === '--allow-young') allowYoung = true
  }
  if (!version) {
    console.error('refresh-rrweb-bundle: --version is required')
    process.exit(2)
  }
  return { version, allowYoung }
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2))
  const here = new URL('.', import.meta.url).pathname
  const vendorDir = resolve(here, '..', 'src', 'vendor')
  const target = join(vendorDir, 'rrweb.umd.min.js')

  const metaRes = await fetch(
    `https://registry.npmjs.org/rrweb/${args.version}`,
  )
  if (!metaRes.ok) {
    console.error(`refresh-rrweb-bundle: registry returned ${metaRes.status}`)
    process.exit(1)
  }
  const meta = (await metaRes.json()) as {
    dist: { tarball: string }
    license: string
  }
  const tsRes = await fetch('https://registry.npmjs.org/rrweb')
  const tsMeta = (await tsRes.json()) as { time: Record<string, string> }
  const releasedAt = tsMeta.time[args.version]
  if (!releasedAt) {
    console.error(
      `refresh-rrweb-bundle: no release timestamp for ${args.version}`,
    )
    process.exit(1)
  }
  const ageDays = Math.floor(
    (Date.parse(new Date().toISOString()) - Date.parse(releasedAt)) /
      (1000 * 60 * 60 * 24),
  )
  if (ageDays < 7 && !args.allowYoung) {
    console.error(
      `refresh-rrweb-bundle: rrweb@${args.version} is ${ageDays}d old; pass --allow-young to override`,
    )
    process.exit(1)
  }

  const tmp = await mkdtemp(join(tmpdir(), 'rrweb-refresh-'))
  try {
    const tgz = join(tmp, 'pkg.tgz')
    const tgzRes = await fetch(meta.dist.tarball)
    if (!tgzRes.ok) {
      console.error(
        `refresh-rrweb-bundle: tarball fetch failed ${tgzRes.status}`,
      )
      process.exit(1)
    }
    await writeFile(tgz, Buffer.from(await tgzRes.arrayBuffer()))
    await $`tar -xzf ${tgz} -C ${tmp}`.quiet()
    const source = join(tmp, 'package', 'dist', 'rrweb.umd.min.cjs')
    const bytes = await Bun.file(source).arrayBuffer()
    await writeFile(target, Buffer.from(bytes))
    console.log(
      `refresh-rrweb-bundle: rrweb@${args.version} -> ${target} (${bytes.byteLength.toLocaleString()} bytes)`,
    )
    console.log(
      'Remember to update src/vendor/README.md with the new version + release date.',
    )
  } finally {
    await rm(tmp, { recursive: true, force: true })
  }
}

await main()
