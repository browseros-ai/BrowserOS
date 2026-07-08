// @license Copyright 2026 BrowserOS
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Fetches the rendered cockpit first-run video + poster into
// `apps/claw-app/public/onboarding/` so the extension build can
// bundle them as static assets. The rendered files are NOT tracked
// in git (see apps/claw-app/.gitignore); this script is how a fresh
// clone gets them.
//
// ─── the full flow ──────────────────────────────────────────────
//
// 1. Source of truth is the Remotion composition in
//    `packages/browseros-agent/packages/onboarding-video/`. That
//    workspace ships:
//      - The composition source (React components under `src/`).
//      - Render scripts (`bun run render` for the MP4,
//        `bun run render:poster` for the PNG poster) that write to
//        the workspace's local `out/` folder (also gitignored).
//
// 2. To change the video, edit the composition in that workspace
//    and re-render locally:
//      $ cd packages/browseros-agent/packages/onboarding-video
//      $ bun run render
//      $ bun run render:poster
//
// 3. To ship the new render to every reader of the extension,
//    publish it as a GitHub Release asset attached to a versioned
//    tag. From the repo root:
//      $ VERSION=v0.2.0   # bump per change; never overwrite an existing tag
//      $ gh release create onboarding-video/$VERSION \
//          --repo browseros-ai/BrowserOS \
//          --target feat/whatever-branch-holds-the-source \
//          --prerelease \
//          --title "Onboarding video $VERSION (preview)" \
//          --notes "Short one-line summary of what changed." \
//          packages/browseros-agent/packages/onboarding-video/out/first-run-demo.mp4 \
//          packages/browseros-agent/packages/onboarding-video/out/first-run-demo-poster.png
//
//    That single command creates the tag AND uploads both assets in
//    one round-trip. `--prerelease` marks it as non-production while
//    the design is still iterating; drop that flag for stable
//    versions. `--target` accepts a branch name or a commit sha
//    (branch names work reliably; short shas sometimes don't).
//
// 4. Bump `RELEASE_TAG` below to the new tag name (e.g. `v0.2.0`)
//    and commit. Every fresh clone that runs `bun run video:fetch`
//    from that commit onward pulls the new video.
//
// 5. To replace an asset within the SAME tag (e.g. a small fix
//    render for the current version) use `gh release upload
//    onboarding-video/$VERSION --repo browseros-ai/BrowserOS
//    --clobber <files>`. Prefer cutting a new version tag over
//    clobbering so historical bundles stay reproducible.
//
// ─── why this pattern ───────────────────────────────────────────
//
// The MP4 is a build artifact of the composition source, not source
// itself. Committing it to git would grow the repo every render.
// GitHub Releases:
//   - are free (no LFS quota), no external infra to manage,
//   - give a stable, cache-friendly URL per tag,
//   - keep every past version accessible for rollback,
//   - never enter git history at all.
//
// Devs cloning the repo can either (a) run
// `packages/onboarding-video/bun run render && cp ...` to render
// their own copy locally (needed to iterate on the composition), or
// (b) run this script to fetch the pinned release.

import { createHash } from 'node:crypto'
import { mkdir, stat, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * The tag name of the GitHub Release that holds the current
 * onboarding video assets. Bump this ONLY when a new tag has been
 * published (see step 3 above).
 */
const RELEASE_TAG = 'onboarding-video/v0.1.0'
const RELEASE_BASE = `https://github.com/browseros-ai/BrowserOS/releases/download/${RELEASE_TAG}`

const ASSETS = [
  { name: 'first-run-demo.mp4', minBytes: 500 * 1024 },
  { name: 'first-run-demo-poster.png', minBytes: 10 * 1024 },
]

const HERE = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = resolve(HERE, '..', 'public', 'onboarding')

await mkdir(OUT_DIR, { recursive: true })

for (const asset of ASSETS) {
  const dest = resolve(OUT_DIR, asset.name)
  // Skip an already-fetched asset that looks intact. Devs re-run
  // this script frequently; a network fetch every time is wasteful.
  const existing = await stat(dest).catch(() => null)
  if (existing && existing.size >= asset.minBytes) {
    console.log(
      `[video:fetch] ${asset.name} already present (${humanBytes(existing.size)}), skipping.`,
    )
    continue
  }
  const url = `${RELEASE_BASE}/${asset.name}`
  console.log(`[video:fetch] downloading ${url}`)
  const res = await fetch(url, { redirect: 'follow' })
  if (!res.ok) {
    console.error(
      `[video:fetch] FAIL ${asset.name}: HTTP ${res.status} ${res.statusText}`,
    )
    console.error(
      `[video:fetch] check that the tag ${RELEASE_TAG} exists at https://github.com/browseros-ai/BrowserOS/releases`,
    )
    process.exit(1)
  }
  const buf = new Uint8Array(await res.arrayBuffer())
  if (buf.byteLength < asset.minBytes) {
    console.error(
      `[video:fetch] FAIL ${asset.name}: expected >= ${humanBytes(asset.minBytes)}, got ${humanBytes(buf.byteLength)}`,
    )
    process.exit(1)
  }
  await writeFile(dest, buf)
  const sha = createHash('sha256').update(buf).digest('hex').slice(0, 12)
  console.log(
    `[video:fetch] wrote ${asset.name} (${humanBytes(buf.byteLength)}, sha256:${sha})`,
  )
}

console.log(`[video:fetch] done. Assets pinned to ${RELEASE_TAG}.`)

function humanBytes(n) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / (1024 * 1024)).toFixed(2)} MB`
}
