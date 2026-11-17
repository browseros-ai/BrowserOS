/**
 * Prints one gap's cases as today's renderer answers them, without writing the committed files:
 *
 *   bun scripts/snapshot-cases/preview.ts <gap file name, e.g. on-top> [case name filter]
 *
 * Only that gap's cases render, so another gap's failing case cannot break this preview.
 */
import type { Gap } from './model'
import { snapshotCaseOutputs } from './write'

const [key, filter = ''] = process.argv.slice(2)
if (!key)
  throw new Error(
    'name the gap: bun scripts/snapshot-cases/preview.ts <gap> [filter]',
  )
const module: Record<string, Gap> = await import(`./${key}.ts`)
const gap = Object.values(module)[0]
if (!gap) throw new Error(`./${key}.ts exports no gap`)
for (const output of snapshotCaseOutputs({ [key]: gap })) {
  const { cases } = JSON.parse(output.text) as {
    cases: {
      name: string
      call: unknown
      text: string | null
      pending?: { text: string; form: unknown }
    }[]
  }
  for (const each of cases) {
    if (!each.name.includes(filter)) continue
    console.log(
      `=== ${each.name}  ${JSON.stringify(each.call)}  (${output.path.split('/').pop()})`,
    )
    console.log(each.text === null ? '(null)' : each.text)
    if (each.pending && each.pending.text !== each.text)
      console.log(
        `--- pending ${JSON.stringify(each.pending.form)}\n${each.pending.text}`,
      )
    else if (each.pending)
      console.log(
        `--- pending: the text above, ${JSON.stringify(each.pending.form)}`,
      )
  }
}
