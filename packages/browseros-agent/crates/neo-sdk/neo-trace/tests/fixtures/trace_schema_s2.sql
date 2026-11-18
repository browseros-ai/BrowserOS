-- Frozen trace schema at 84da7b741; retained to prove adoption and old-build parity.
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  client TEXT,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  last_activity_at INTEGER NOT NULL,
  private_since INTEGER,
  private_reason TEXT
);
CREATE INDEX IF NOT EXISTS sessions_by_activity ON sessions(last_activity_at, id);
CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  status TEXT NOT NULL,
  effects TEXT,
  error TEXT,
  request_id TEXT,
  host_data TEXT,
  code TEXT,
  code_truncated INTEGER NOT NULL DEFAULT 0,
  code_withheld TEXT,
  result TEXT,
  result_truncated INTEGER NOT NULL DEFAULT 0,
  replay TEXT,
  replay_until INTEGER,
  operation_count INTEGER NOT NULL DEFAULT 0,
  trace_gaps INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS runs_by_session ON runs(session_id, started_at);
CREATE INDEX IF NOT EXISTS runs_by_start ON runs(started_at, id);
-- Not unique on purpose: admission refuses a repeated request id, and a unique index here
-- would fail a whole writer batch (and lose its records) if two runs ever raced past it.
CREATE INDEX IF NOT EXISTS runs_by_request ON runs(request_id, session_id)
  WHERE request_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS operations (
  run_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  parent_id TEXT,
  session_id TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  attempt INTEGER,
  effect TEXT NOT NULL,
  page_id INTEGER,
  tab_id INTEGER,
  url TEXT,
  title TEXT,
  started_at INTEGER NOT NULL,
  sent_at INTEGER,
  ended_at INTEGER,
  outcome TEXT,
  delivery TEXT,
  args TEXT,
  args_truncated INTEGER NOT NULL DEFAULT 0,
  result TEXT,
  result_truncated INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  PRIMARY KEY (run_id, seq)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS operations_by_session ON operations(session_id, started_at, run_id, seq);
