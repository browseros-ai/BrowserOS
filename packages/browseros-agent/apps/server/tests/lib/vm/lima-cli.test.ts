/**
 * @license
 * Copyright 2025 BrowserOS
 */

import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  mock,
  spyOn,
} from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { logger } from '../../../src/lib/logger'
import { LimaCommandError, VmNotReadyError } from '../../../src/lib/vm/errors'
import { LimaCli } from '../../../src/lib/vm/lima-cli'
import { VM_TELEMETRY_EVENTS } from '../../../src/lib/vm/telemetry'
import { fakeLimactl } from '../../__helpers__/fake-limactl'
import { fakeSsh } from '../../__helpers__/fake-ssh'

describe('LimaCli', () => {
  let tempDir: string
  let logPath: string
  let limaHome: string

  beforeEach(async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'lima-cli-test-'))
    logPath = join(tempDir, 'calls.log')
    limaHome = join(tempDir, 'lima-home')
  })

  afterEach(async () => {
    mock.restore()
    await rm(tempDir, { recursive: true, force: true })
  })

  it('parses limactl list JSON output', async () => {
    const limactlPath = await fakeLimactl(
      {
        list: {
          stdout: JSON.stringify([
            {
              name: 'browseros-vm',
              status: 'Running',
              dir: '/lima/browseros-vm',
            },
          ]),
        },
      },
      logPath,
    )
    const cli = new LimaCli({ limactlPath, limaHome })

    await expect(cli.list()).resolves.toEqual([
      { name: 'browseros-vm', status: 'Running', dir: '/lima/browseros-vm' },
    ])
  })

  it('returns an empty VM list when limactl prints no output', async () => {
    const limactlPath = await fakeLimactl({ list: { stdout: '' } }, logPath)
    const cli = new LimaCli({ limactlPath, limaHome })

    await expect(cli.list()).resolves.toEqual([])
  })

  it('creates VMs with LIMA_HOME and the expected argv', async () => {
    const limactlPath = await fakeLimactl({ create: {} }, logPath)
    const cli = new LimaCli({ limactlPath, limaHome })

    await cli.create('browseros-vm', '/tmp/browseros-vm.yaml')

    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      'ARGS:create --tty=false --name=browseros-vm /tmp/browseros-vm.yaml',
    )
    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      `LIMA_HOME:${limaHome}`,
    )
  })

  it('starts VMs with tty disabled', async () => {
    const limactlPath = await fakeLimactl({ start: {} }, logPath)
    const cli = new LimaCli({ limactlPath, limaHome })

    await cli.start('browseros-vm')

    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      'ARGS:start --tty=false browseros-vm',
    )
  })

  it('throws LimaCommandError with stderr on non-zero exit', async () => {
    const limactlPath = await fakeLimactl(
      { start: { stderr: 'cannot start', exit: 2 } },
      logPath,
    )
    const cli = new LimaCli({ limactlPath, limaHome })

    const error = await cli.start('browseros-vm').catch((err) => err)

    expect(error).toBeInstanceOf(LimaCommandError)
    expect(error.exitCode).toBe(2)
    expect(error.stderr).toBe('cannot start')
  })

  it('does not log limactl stderr chunks by default', async () => {
    const debug = spyOn(logger, 'debug').mockImplementation(() => {})
    const limactlPath = await fakeLimactl(
      { start: { stderr: 'boot noise\n' } },
      logPath,
    )
    const cli = new LimaCli({ limactlPath, limaHome })

    await cli.start('browseros-vm')

    expect(
      debug.mock.calls.some(
        ([message]) => message === VM_TELEMETRY_EVENTS.limaStderrChunk,
      ),
    ).toBe(false)
  })

  it('stops and deletes VMs', async () => {
    const limactlPath = await fakeLimactl({ stop: {}, delete: {} }, logPath)
    const cli = new LimaCli({ limactlPath, limaHome })

    await cli.stop('browseros-vm')
    await cli.delete('browseros-vm')

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain('ARGS:stop browseros-vm')
    expect(log).toContain('ARGS:delete --force browseros-vm')
  })

  it('runs shell commands and streams stdout and stderr', async () => {
    const sshPath = await fakeSsh({ stdout: 'out\n', stderr: 'err\n' }, logPath)
    const sshConfig = join(limaHome, 'browseros-vm', 'ssh.config')
    await mkdir(join(limaHome, 'browseros-vm'), { recursive: true })
    await writeFile(sshConfig, '')
    const cli = new LimaCli({ limactlPath: 'unused', limaHome, sshPath })
    const lines: string[] = []

    await expect(
      cli.shell('browseros-vm', ['nerdctl', 'ps'], {
        onStdout: (line) => lines.push(`stdout:${line}`),
        onStderr: (line) => lines.push(`stderr:${line}`),
      }),
    ).resolves.toBe(0)

    expect(lines).toContain('stdout:out')
    expect(lines).toContain('stderr:err')
    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      `ARGS:-F ${sshConfig} lima-browseros-vm 'nerdctl' 'ps'`,
    )
  })

  it('shell-quotes remote commands to preserve argument boundaries', async () => {
    const sshPath = await fakeSsh({}, logPath)
    const sshConfig = join(limaHome, 'browseros-vm', 'ssh.config')
    await mkdir(join(limaHome, 'browseros-vm'), { recursive: true })
    await writeFile(sshConfig, '')
    const cli = new LimaCli({ limactlPath: 'unused', limaHome, sshPath })

    await expect(
      cli.shell('browseros-vm', ['sh', '-lc', "echo 'boundary ok'"]),
    ).resolves.toBe(0)

    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      `ARGS:-F ${sshConfig} lima-browseros-vm 'sh' '-lc' 'echo '\\''boundary ok'\\'''`,
    )
  })

  it('ignores shell stderr when no stderr stream handler is provided', async () => {
    const sshConfig = join(limaHome, 'browseros-vm', 'ssh.config')
    await mkdir(join(limaHome, 'browseros-vm'), { recursive: true })
    await writeFile(sshConfig, '')
    const spawn = spyOn(Bun, 'spawn')
    spawn.mockImplementation(
      () =>
        ({
          stdout: null,
          stderr: null,
          exited: Promise.resolve(0),
        }) as never,
    )
    const cli = new LimaCli({ limactlPath: 'limactl', limaHome })

    await expect(
      cli.shell('browseros-vm', ['true'], {
        onStdout: () => {},
      }),
    ).resolves.toBe(0)

    expect(spawn).toHaveBeenCalledWith(
      ['ssh', '-F', sshConfig, 'lima-browseros-vm', "'true'"],
      expect.objectContaining({
        stdout: 'pipe',
        stderr: 'ignore',
      }),
    )
  })

  it('throws VmNotReadyError when ssh.config is missing', async () => {
    const cli = new LimaCli({ limactlPath: 'limactl', limaHome })
    const error = await cli.shell('browseros-vm', ['true']).catch((err) => err)
    expect(error).toBeInstanceOf(VmNotReadyError)
  })
})
