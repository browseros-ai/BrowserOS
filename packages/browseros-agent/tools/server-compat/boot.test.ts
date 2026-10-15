/** Fake executables reproduce startup/signal overlap without an installed browser, Cargo or owner state. */
import { expect, test } from 'bun:test'
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

// A red regression can create a leaked fixture child; never leave that owned test process behind.
async function terminateLeakedFixture(marker: string) {
  if (await Bun.file(marker).exists()) {
    const { pid } = JSON.parse(await readFile(marker, 'utf8')) as {
      pid: number
    }
    try {
      process.kill(-pid, 'SIGKILL')
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error
    }
  }
}

test('tests that shutdown during CDP readiness cannot launch a detached server afterward', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'server-compat-startup-test-'))
  const browser = join(fixture, 'browser')
  const server = join(fixture, 'server')
  const marker = join(fixture, 'server-started.json')
  await writeFile(
    browser,
    `#!${process.execPath}\nconst port = Number(process.argv.find(arg => arg.startsWith('--remote-debugging-port=')).split('=')[1]); Bun.serve({hostname:'127.0.0.1',port,fetch:()=>Response.json({webSocketDebuggerUrl:'ws://127.0.0.1:1/unused'})});\n`,
  )
  await writeFile(
    server,
    `#!${process.execPath}\nawait Bun.write(${JSON.stringify(marker)}, JSON.stringify({pid:process.pid})); Bun.serve({hostname:'127.0.0.1',port:0,fetch:()=>new Response('unexpected server')});\n`,
  )
  await Promise.all([chmod(browser, 0o700), chmod(server, 0o700)])
  const coordinator = `
    import {bootServer,runCli} from ${JSON.stringify(join(import.meta.dir, 'boot.ts'))};
    const original = globalThis.fetch;
    globalThis.fetch = async (...args) => {
      const response = await original(...args);
      if (String(args[0]).endsWith('/json/version') && response.ok) process.emit('SIGTERM');
      return response;
    };
    await runCli(async () => { const server = await bootServer('rust'); await server.stop(); });
  `
  const child = Bun.spawn([process.execPath, '-e', coordinator], {
    env: {
      ...process.env,
      BROWSEROS_BINARY: browser,
      SERVER_RUST_BINARY: server,
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const timer = setTimeout(() => child.kill('SIGKILL'), 10_000)
  try {
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])
    if (code !== 130)
      throw new Error(
        `Expected signal exit 130, got ${code}: ${stdout}${stderr}`,
      )
    // Give a wrongly detached child enough time to publish its marker, then clean it up even on red.
    await Bun.sleep(100)
    expect(await Bun.file(marker).exists()).toBe(false)
  } finally {
    clearTimeout(timer)
    await terminateLeakedFixture(marker)
    if (child.exitCode === null) {
      child.kill()
      await child.exited
    }
    await rm(fixture, { recursive: true, force: true })
  }
})
