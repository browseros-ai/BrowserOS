/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * Tests for tri tools — Issue #7
 *
 * Tests:
 * 1. tri_run: executes tri commands and returns structured result
 * 2. tri_spec_edit: writes spec file and optionally tests
 * 3. tri_experience_read: reads .trinity files
 * 4. Error format: {ok: false, reason: "..."}
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { TriClient } from '../clients/tri-client.js'

let tempDir: string
let tri: TriClient

beforeAll(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'trios-tri-test-'))
  tri = new TriClient('tri', tempDir)

  // Create .trinity/experience directory with test files
  const expDir = join(tempDir, '.trinity', 'experience')
  await mkdir(expDir, { recursive: true })
  await writeFile(
    join(expDir, 'entry-1.trinity'),
    '[2026-01-01T00:00:00Z] First experience entry\nTest content 1',
  )
  await writeFile(
    join(expDir, 'entry-2.trinity'),
    '[2026-01-02T00:00:00Z] Second experience entry\nTest content 2',
  )
  await writeFile(
    join(expDir, 'entry-3.trinity'),
    '[2026-01-03T00:00:00Z] Third experience entry\nTest content 3',
  )
  // Non-trinity file (should be ignored)
  await writeFile(join(expDir, 'notes.txt'), 'Not a trinity file')
})

afterAll(async () => {
  await rm(tempDir, { recursive: true, force: true })
})

// --- tri_run ---

describe('tri_run', () => {
  test('tri --help returns ok with exit code 0', async () => {
    const result = await tri.run(['--help'])

    expect(result.ok).toBe(true)
    expect(result.exitCode).toBe(0)
    expect(result.stdout).toContain('PHI LOOP')
    expect(result.command).toBe('tri --help')
  })

  test('tri status returns structured result', async () => {
    const result = await tri.run(['status'])

    // status may succeed or fail depending on project state
    expect(result).toHaveProperty('ok')
    expect(result).toHaveProperty('exitCode')
    expect(result).toHaveProperty('stdout')
    expect(result).toHaveProperty('stderr')
    expect(result).toHaveProperty('command')
    expect(result.command).toBe('tri status')
  })

  test('tri with invalid command returns non-zero exit code', async () => {
    const result = await tri.run(['nonexistent-command-xyz'])

    expect(result.ok).toBe(false)
    expect(result.exitCode).not.toBe(0)
  })

  test('result has no null fields', async () => {
    const result = await tri.run(['--help'])

    expect(result.ok).not.toBeNull()
    expect(result.exitCode).not.toBeNull()
    expect(result.stdout).not.toBeNull()
    expect(result.stderr).not.toBeNull()
    expect(result.command).not.toBeNull()
  })
})

// --- tri_spec_edit ---

describe('tri_spec_edit', () => {
  test('writes spec file without test', async () => {
    const specPath = 'test-spec.t27'
    const content = '# Test Spec\n## L1: Build\n- [ ] Item 1'

    const result = await tri.specEdit(specPath, content, false)

    expect(result.ok).toBe(true)
    expect(result.reason).toContain('written')
    expect(result.specPath).toBe(specPath)

    // Verify file was written
    const file = Bun.file(join(tempDir, specPath))
    const text = await file.text()
    expect(text).toBe(content)
  })

  test('writes spec file and runs test (may fail if tri test not applicable)', async () => {
    const specPath = 'test-spec-with-test.t27'
    const content = '# Test Spec 2\n## L1: Build\n- [ ] Item 1'

    const result = await tri.specEdit(specPath, content, true)

    expect(result).toHaveProperty('ok')
    expect(result).toHaveProperty('specPath')
    expect(result.specPath).toBe(specPath)
    // testPassed may be true or false depending on tri test behavior
    if (result.testPassed !== undefined) {
      expect(typeof result.testPassed).toBe('boolean')
    }
  })

  test('specEdit result has no null fields', async () => {
    const result = await tri.specEdit('null-check.t27', 'content', false)

    expect(result.ok).not.toBeNull()
    expect(result.reason).not.toBeNull()
    expect(result.specPath).not.toBeNull()
  })
})

// --- tri_experience_read ---

describe('tri_experience_read', () => {
  test('reads experience entries from .trinity directory', async () => {
    const entries = await tri.readExperiences(
      10,
      join(tempDir, '.trinity', 'experience'),
    )

    expect(entries.length).toBe(3) // 3 .trinity files, notes.txt ignored
  })

  test('entries have correct structure', async () => {
    const entries = await tri.readExperiences(
      10,
      join(tempDir, '.trinity', 'experience'),
    )

    for (const entry of entries) {
      expect(entry.fileName).toBeTruthy()
      expect(entry.fileName.endsWith('.trinity')).toBe(true)
      expect(entry.content).toBeTruthy()
      expect(entry.modified).toBeTruthy()
    }
  })

  test('respects count limit', async () => {
    const entries = await tri.readExperiences(
      2,
      join(tempDir, '.trinity', 'experience'),
    )

    expect(entries.length).toBe(2)
  })

  test('returns empty array for nonexistent directory', async () => {
    const entries = await tri.readExperiences(5, '/nonexistent/path')

    expect(entries).toEqual([])
  })

  test('entries have no null fields', async () => {
    const entries = await tri.readExperiences(
      10,
      join(tempDir, '.trinity', 'experience'),
    )

    for (const entry of entries) {
      expect(entry.fileName).not.toBeNull()
      expect(entry.content).not.toBeNull()
      expect(entry.modified).not.toBeNull()
    }
  })

  test('ignores non-.trinity files', async () => {
    const entries = await tri.readExperiences(
      10,
      join(tempDir, '.trinity', 'experience'),
    )

    const fileNames = entries.map((e) => e.fileName)
    expect(fileNames).not.toContain('notes.txt')
  })
})

// --- tri_client availability ---

describe('tri_client availability', () => {
  test('isAvailable returns true when tri is installed', async () => {
    const available = await tri.isAvailable()
    // tri is installed on this system
    expect(available).toBe(true)
  })

  test('isAvailable returns false for nonexistent CLI', async () => {
    const badClient = new TriClient('nonexistent-tri-cli-xyz', tempDir)
    const available = await badClient.isAvailable()
    expect(available).toBe(false)
  })
})
