/** Neo: Owns one private browser process group; no protocol helper reaches a test body. */
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

function signalGroup(pid, signal) {
  try {
    process.kill(-pid, signal)
    return true
  } catch (error) {
    if (error.code === 'ESRCH') return false
    throw error
  }
}

async function stopGroup(pid) {
  signalGroup(pid, 'SIGTERM')
  for (let attempt = 0; attempt < 40; attempt++) {
    if (!signalGroup(pid, 0)) return
    await sleep(50)
  }
  signalGroup(pid, 'SIGKILL')
  for (let attempt = 0; attempt < 60; attempt++) {
    if (!signalGroup(pid, 0)) return
    await sleep(50)
  }
  throw new Error('Owned browser process group survived cleanup')
}

/** Parent fallback for a worker that died before fixture teardown. */
export async function cleanupWorkers(outputDir) {
  for (const entry of await fs.readdir(outputDir, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('worker-')) continue
    const workerDir = path.join(outputDir, entry.name)
    let owner
    try {
      await fs.access(path.join(workerDir, 'cleanup.json'))
      continue
    } catch (error) {
      if (error.code !== 'ENOENT') throw error
    }
    try {
      owner = JSON.parse(
        await fs.readFile(path.join(workerDir, 'owner.json'), 'utf8'),
      )
    } catch (error) {
      if (error.code === 'ENOENT') continue
      throw error
    }
    if (signalGroup(owner.pid, 0)) {
      // Revalidate the private profile before touching a PID retained after a worker exited.
      const processes = execFileSync('ps', ['-axo', 'pgid=,command='], {
        encoding: 'utf8',
      })
      const owned = processes
        .split('\n')
        .some(
          (line) =>
            line.trim().startsWith(`${owner.pid} `) &&
            line.includes(owner.profile),
        )
      if (!owned)
        throw new Error(`Cannot revalidate browser group ${owner.pid}`)
      await stopGroup(owner.pid)
    }
    await fs.rm(owner.profile, { recursive: true, force: true })
    await fs.writeFile(
      path.join(workerDir, 'cleanup.json'),
      JSON.stringify({ pid: owner.pid, parentFallback: true }),
    )
  }
}

export async function launchBrowser(executable, outputDir) {
  await fs.mkdir(outputDir, { recursive: true })
  const profile = await fs.mkdtemp(path.join(outputDir, 'profile-'))
  const stdout = await fs.open(path.join(outputDir, 'browser.stdout.log'), 'wx')
  const stderr = await fs.open(path.join(outputDir, 'browser.stderr.log'), 'wx')
  let child
  let exit
  let groupVerified = false
  let closing
  let exited
  const close = () =>
    (closing ??= (async () => {
      if (child?.pid && groupVerified) {
        await stopGroup(child.pid)
        await Promise.race([exited, sleep(3000, undefined, { ref: false })])
        await fs.writeFile(
          path.join(outputDir, 'cleanup.json'),
          JSON.stringify({ pid: child.pid, exit }),
        )
      } else if (child?.pid && !exit) {
        child.kill('SIGKILL')
      }
      await fs.rm(profile, { recursive: true, force: true })
    })())
  try {
    child = spawn(
      executable,
      [
        '--headless=new',
        '--disable-browseros-server',
        '--disable-browseros-extensions',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        '--no-first-run',
        '--lang=en-US',
        '--window-size=1280,720',
        '--force-device-scale-factor=1',
        'about:blank',
      ],
      { detached: true, stdio: ['ignore', stdout.fd, stderr.fd] },
    )
    exited = new Promise((resolve) => {
      child.once('exit', (code, signal) => {
        exit = { code, signal }
        resolve()
      })
      child.once('error', (error) => {
        exit = { error: error.message }
        resolve()
      })
    })
    if (!child.pid) throw new Error('Browser did not start')
    groupVerified =
      execFileSync('ps', ['-o', 'pgid=', '-p', String(child.pid)], {
        encoding: 'utf8',
      }).trim() === String(child.pid)
    if (!groupVerified) throw new Error('Browser did not own its process group')
    await fs.writeFile(
      path.join(outputDir, 'owner.json'),
      JSON.stringify({ pid: child.pid, profile }),
    )
    const deadline = Date.now() + 30_000
    while (Date.now() < deadline) {
      if (exit)
        throw new Error(`Browser exited at startup: ${JSON.stringify(exit)}`)
      try {
        const [port] = (
          await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')
        ).split('\n')
        if (/^\d+$/.test(port) && Number(port) > 0)
          return { port: Number(port), close }
      } catch (error) {
        if (error.code !== 'ENOENT') throw error
      }
      await sleep(50)
    }
    throw new Error('Browser did not publish its CDP port')
  } catch (error) {
    await close()
    throw error
  } finally {
    await Promise.all([stdout.close(), stderr.close()])
  }
}
