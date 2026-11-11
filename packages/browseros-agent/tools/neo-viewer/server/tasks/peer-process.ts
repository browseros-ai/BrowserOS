/**
 * Finds the local process at the other end of an agent's TCP connection, for the task rule's
 * second case (`task-rule.ts`). The viewer binds 127.0.0.1, so every caller is a process on this
 * machine (a container's call arrives from Docker Desktop's forwarder).
 *
 * One socket-table read (`netstat -anv` on macOS, about 30 ms; `ss -tnp` on Linux) names the
 * process holding the connection's client port; `ps` adds its start time, so a reused pid is a
 * new process. Results are cached per connection (address and port) while the connection is in
 * use. A client that closes its connection before the lookup finishes is not found; the rule
 * then falls back to its third case.
 */

import type { ProcessLookup } from './task-rule'

export interface Peer {
  address: string
  port: number
}

/** The proxy's view of the finder; tests stand in their own. */
export interface ProcessFinder {
  find(peer: Peer | null): {
    lookup: Promise<ProcessLookup>
    release: () => void
  }
}

export interface PeerProcessOptions {
  /** The viewer's own listening port: the far end of every agent connection. */
  listenPort: number
  platform?: NodeJS.Platform
  /** Runs a command and returns its stdout; rejects when it cannot run or exits non-zero. */
  run?: (command: string, args: string[]) => Promise<string>
  /** How long a connection's answer is reused after its last call ends. */
  reuseMs?: number
  /** Longest wait for an answer; slower lookups count as not found. */
  timeoutMs?: number
  now?: () => number
}

interface Cached {
  lookup: Promise<ProcessLookup>
  /** Calls on this connection still going. */
  active: number
  idleSince: number
}

const LOOKUP_TIMEOUT_MS = 500
const CONNECTION_REUSE_MS = 3000

export class PeerProcessFinder implements ProcessFinder {
  private readonly connections = new Map<string, Cached>()
  private readonly processes = new Map<
    number,
    Promise<{ startedAt: string | null; command: string | null }>
  >()
  private readonly platform: NodeJS.Platform
  private readonly run: NonNullable<PeerProcessOptions['run']>
  private readonly reuseMs: number
  private readonly timeoutMs: number
  private readonly now: () => number

  constructor(private readonly options: PeerProcessOptions) {
    this.platform = options.platform ?? process.platform
    this.run = options.run ?? runCommand
    this.reuseMs = options.reuseMs ?? CONNECTION_REUSE_MS
    this.timeoutMs = options.timeoutMs ?? LOOKUP_TIMEOUT_MS
    this.now = options.now ?? Date.now
  }

  /**
   * The process behind `peer`, counted as in use until the returned `release` runs. Ports are
   * reused once a connection closes, so a cached answer is trusted only while a call is going
   * on that connection or shortly after one ended (`reuseMs`).
   */
  find(peer: Peer | null): {
    lookup: Promise<ProcessLookup>
    release: () => void
  } {
    if (!peer)
      return {
        lookup: Promise.resolve({ found: false, reason: 'no peer address' }),
        release: () => {},
      }
    const key = `${peer.address}:${peer.port}`
    let cached = this.connections.get(key)
    if (
      cached &&
      cached.active === 0 &&
      this.now() - cached.idleSince > this.reuseMs
    ) {
      cached = undefined
    }
    if (!cached) {
      cached = { lookup: this.lookup(peer), active: 0, idleSince: this.now() }
      this.connections.set(key, cached)
    }
    const entry = cached
    entry.active++
    let released = false
    return {
      lookup: entry.lookup,
      release: () => {
        if (released) return
        released = true
        entry.active--
        entry.idleSince = this.now()
        this.forgetIdle()
      },
    }
  }

  private forgetIdle(): void {
    const now = this.now()
    for (const [key, entry] of this.connections) {
      if (entry.active === 0 && now - entry.idleSince > this.reuseMs)
        this.connections.delete(key)
    }
  }

  private async lookup(peer: Peer): Promise<ProcessLookup> {
    let timer: ReturnType<typeof setTimeout> | undefined
    const timeout = new Promise<ProcessLookup>((resolve) => {
      timer = setTimeout(
        () =>
          resolve({
            found: false,
            reason: `process lookup took over ${this.timeoutMs} ms`,
          }),
        this.timeoutMs,
      )
    })
    try {
      return await Promise.race([this.lookupNow(peer), timeout])
    } finally {
      clearTimeout(timer)
    }
  }

  private async lookupNow(peer: Peer): Promise<ProcessLookup> {
    let pid: number | null
    try {
      pid = await this.socketOwner(peer)
    } catch (error) {
      return {
        found: false,
        reason: `socket table unreadable: ${String(error)}`,
      }
    }
    if (pid === null) {
      return {
        found: false,
        reason: 'connection not in the socket table (closed already?)',
      }
    }
    const details = await this.processDetails(pid)
    return { found: true, pid, ...details }
  }

  private async socketOwner(peer: Peer): Promise<number | null> {
    if (this.platform === 'darwin') {
      const table = await this.run('netstat', ['-anv', '-p', 'tcp'])
      return ownerFromNetstat(table, peer, this.options.listenPort)
    }
    if (this.platform === 'linux') {
      const table = await this.run('ss', ['-Htnp', 'state', 'established'])
      return ownerFromSs(table, peer, this.options.listenPort)
    }
    throw new Error(`no socket table reader for ${this.platform}`)
  }

  private processDetails(
    pid: number,
  ): Promise<{ startedAt: string | null; command: string | null }> {
    let details = this.processes.get(pid)
    if (!details) {
      details = this.run('ps', [
        '-o',
        'lstart=',
        '-o',
        'comm=',
        '-p',
        String(pid),
      ])
        .then(parsePs)
        .catch(() => ({ startedAt: null, command: null }))
      this.processes.set(pid, details)
      // A pid's start time never changes, but the pid can be reused; keep answers briefly.
      setTimeout(() => this.processes.delete(pid), 60_000).unref()
    }
    return details
  }
}

/**
 * macOS `netstat -anv -p tcp`: the ESTABLISHED tcp4 row whose local end is the agent's port and
 * whose foreign end is ours; `process:pid` follows rxbytes, txbytes, rhiwat and shiwat.
 */
export function ownerFromNetstat(
  table: string,
  peer: Peer,
  listenPort: number,
): number | null {
  const local = `${peer.address}.${peer.port}`
  const foreign = `${peer.address}.${listenPort}`
  for (const line of table.split('\n')) {
    const match =
      /^tcp4\s+\d+\s+\d+\s+(\S+)\s+(\S+)\s+ESTABLISHED\s+\d+\s+\d+\s+\d+\s+\d+\s+(.*?):(\d+)\s/.exec(
        line,
      )
    if (match && match[1] === local && match[2] === foreign)
      return Number(match[4])
  }
  return null
}

/** Linux `ss -Htnp state established`: `Recv-Q Send-Q Local Peer users:(("name",pid=N,fd=M))`. */
export function ownerFromSs(
  table: string,
  peer: Peer,
  listenPort: number,
): number | null {
  const local = `${peer.address}:${peer.port}`
  const foreign = `${peer.address}:${listenPort}`
  for (const line of table.split('\n')) {
    const fields = line.trim().split(/\s+/)
    if (fields[2] === local && fields[3] === foreign) {
      const pid = /pid=(\d+)/.exec(line)
      return pid ? Number(pid[1]) : null
    }
  }
  return null
}

/** `ps -o lstart= -o comm=`: a 24-character start time, then the command. */
export function parsePs(output: string): {
  startedAt: string | null
  command: string | null
} {
  const match =
    /^\s*(\w{3} \w{3} [ \d]\d \d\d:\d\d:\d\d \d{4})\s+(.*?)\s*$/.exec(
      output.split('\n')[0] ?? '',
    )
  return match
    ? { startedAt: match[1] ?? null, command: match[2] || null }
    : { startedAt: null, command: null }
}

const runCommand: NonNullable<PeerProcessOptions['run']> = async (
  command,
  args,
) => {
  const child = Bun.spawn([command, ...args], {
    stdout: 'pipe',
    stderr: 'ignore',
  })
  const [stdout, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    child.exited,
  ])
  if (exitCode !== 0) throw new Error(`${command} exited ${exitCode}`)
  return stdout
}
