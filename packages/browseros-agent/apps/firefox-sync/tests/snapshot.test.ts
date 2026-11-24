import { Database } from 'bun:sqlite'
import { afterEach, describe, expect, test } from 'bun:test'
import { existsSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { withSqliteSnapshot } from '../src/firefox/snapshot'
import { tempProfile } from './fixtures'

const dirs: string[] = []
function profile() {
  const dir = tempProfile()
  dirs.push(dir)
  return dir
}
const open: Database[] = []
afterEach(() => {
  for (const db of open.splice(0)) db.close()
  for (const dir of dirs.splice(0))
    rmSync(dir, { recursive: true, force: true })
})

describe('withSqliteSnapshot', () => {
  test('sees writes that still sit in the WAL of a database held open', () => {
    const dir = profile()
    const live = new Database(join(dir, 'data.sqlite'))
    open.push(live)
    live.run('PRAGMA journal_mode = WAL')
    live.run('PRAGMA wal_autocheckpoint = 0')
    live.run('CREATE TABLE t (v TEXT)')
    live.run("INSERT INTO t VALUES ('in-wal')")
    expect(existsSync(join(dir, 'data.sqlite-wal'))).toBe(true)

    const rows = withSqliteSnapshot(dir, 'data.sqlite', (db) =>
      db.query('SELECT v FROM t').all(),
    )
    expect(rows).toEqual([{ v: 'in-wal' }])
  })

  test('reads a database that has no WAL', () => {
    const dir = profile()
    const db = new Database(join(dir, 'data.sqlite'))
    db.run('CREATE TABLE t (v INTEGER)')
    db.run('INSERT INTO t VALUES (7)')
    db.close()
    expect(
      withSqliteSnapshot(dir, 'data.sqlite', (copy) =>
        copy.query('SELECT v FROM t').get(),
      ),
    ).toEqual({ v: 7 })
  })

  test('names the missing file', () => {
    expect(() => withSqliteSnapshot(profile(), 'nope.sqlite', () => 0)).toThrow(
      'nope.sqlite not found',
    )
  })
})
