/** Neo-specific expectations run only on the real product path. This suite has no oracle or
 * differential normalization, so it cannot be mistaken for another conformance tier. */
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { start } from './environment.mjs'
import { connectNeo, runNeo } from './runner-neo.mjs'
export async function behavior({
  server,
  output,
  browser,
  neoBinary,
  gitSha,
  ports,
  filter,
  signal,
}) {
  mkdirSync(join(output, 'logs'), { recursive: true })
  const env = await start({
    ports,
    browser,
    neoBinary,
    assetsDir: join(server, 'tests/behavior/pages'),
    runDir: output,
    gitSha,
    signal,
  })
  let client
  const results = []
  try {
    client = await connectNeo(env, signal)
    for (const name of readdirSync(join(server, 'tests/behavior'))
      .filter((n) => n.endsWith('.js') && (!filter || n.includes(filter)))
      .sort()) {
      signal?.throwIfAborted()
      const code = `const origin=${JSON.stringify(env.server.CROSS_PROCESS_PREFIX)};
const initial=new Set(context.pages());
let record;
try {
  await (async(page)=>{
${readFileSync(join(server, 'tests/behavior', name), 'utf8')}
})(await context.newPage());
  record={behaviorPassed:true};
} catch(error) {
  await Promise.resolve();
  record={behaviorPassed:false,error:{name:error.name,message:String(error.message).slice(0,8192),code:error.code,owner:error.owner,delivery:error.delivery}};
}
record.cleanupErrors=[];
for(const opened of context.pages())if(!initial.has(opened)) {
  try {await opened.close()}catch(error){record.cleanupErrors.push(String(error.message))}
}
return record;`

      const file = join(output, name)
      writeFileSync(file, code)
      const record = await runNeo({
        file,
        row: { id: name },
        env,
        client,
        output: join(output, `${name}.json`),
        requestId: `behavior/${Date.now()}/${name}`,
        taskId: 'behavior',
        signal,
      })
      results.push({ name, passed: record.behaviorPassed === true, record })
      console.log(
        `${name}: ${record.behaviorPassed === true ? 'PASS' : 'FAIL'} ${record.error?.message || record.transport?.error || ''}`,
      )
    }
  } finally {
    try {
      await client?.close()
    } finally {
      await env.stop()
    }
  }
  writeFileSync(join(output, 'report.json'), JSON.stringify(results, null, 2))
  if (!results.length) throw new Error('no behavior checks selected')
  return results.every((r) => r.passed)
}
