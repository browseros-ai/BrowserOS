/**
 * The web page, bundled in memory when the viewer starts: `web/index.html` and what it links
 * (React TSX, Tailwind CSS), through `Bun.build`. Tailwind is compiled the way
 * `@tailwindcss/vite` does it in `apps/claw-app` (the same 4.x packages): `@tailwindcss/node`
 * compiles the stylesheet and `@tailwindcss/oxide` scans `web/` for the classes it uses.
 *
 * Bun's own route-level HTML bundling would need `bun-plugin-tailwind`, whose peer dependency on
 * the `bun` npm package puts another Bun and its platform binaries in the lockfile.
 */

import { dirname, join } from 'node:path'
import { compile } from '@tailwindcss/node'
import { Scanner } from '@tailwindcss/oxide'
import type { BunPlugin } from 'bun'

const WEB_DIR = join(import.meta.dir, '..', 'web')

/** One built file, served at `/<name>`; `index.html` is also served at `/`. */
export interface PageFile {
  body: Blob
  type: string
}

export async function buildPage(): Promise<Map<string, PageFile>> {
  const result = await Bun.build({
    entrypoints: [join(WEB_DIR, 'index.html')],
    plugins: [tailwind],
    target: 'browser',
    define: {
      'process.env.NODE_ENV': JSON.stringify(
        process.env.NODE_ENV ?? 'development',
      ),
    },
  })
  if (!result.success) {
    throw new AggregateError(
      result.logs,
      'neo-viewer: the web page did not build',
    )
  }
  const files = new Map<string, PageFile>()
  for (const output of result.outputs) {
    files.set(output.path.replace(/^\.?\//, ''), {
      body: output,
      type: output.type,
    })
  }
  return files
}

const tailwind: BunPlugin = {
  name: 'tailwind',
  setup(build) {
    build.onLoad({ filter: /\.css$/ }, async ({ path }) => {
      const compiler = await compile(await Bun.file(path).text(), {
        base: dirname(path),
        from: path,
        onDependency: () => {},
      })
      const root =
        compiler.root === 'none'
          ? []
          : compiler.root === null
            ? [{ base: WEB_DIR, pattern: '**/*', negated: false }]
            : [{ ...compiler.root, negated: false }]
      const scanner = new Scanner({ sources: [...root, ...compiler.sources] })
      return { contents: compiler.build(scanner.scan()), loader: 'css' }
    })
  },
}
