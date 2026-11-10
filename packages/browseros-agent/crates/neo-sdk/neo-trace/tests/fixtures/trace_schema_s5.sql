-- Frozen trace schema at a4279a305; retained to prove adoption and old-build parity.
CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  task_id TEXT,
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
  trace_gaps INTEGER NOT NULL DEFAULT 0,
  -- Ended, and every screenshot job it made answered. Only the writer sets it, in the batch
  -- that stores the last answer (or the end, when nothing is pending).
  complete INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS runs_by_start ON runs(started_at, id);
-- Not unique on purpose: admission keeps a request id from running twice (per task when the run
-- has one, else across Neo), and a unique index here would fail a whole writer batch (and lose
-- its records) if two runs ever raced past it.
CREATE INDEX IF NOT EXISTS runs_by_request ON runs(request_id, task_id)
  WHERE request_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS runs_by_task ON runs(task_id, started_at)
  WHERE task_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS operations (
  run_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  parent_id TEXT,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  attempt INTEGER,
  effect TEXT NOT NULL,
  -- The tab the operation targeted. page_id, tab_id, url and title are the page after it
  -- (`page_after`), as the browser reported it.
  target_tab_id INTEGER,
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
  -- Its call-log lines, a JSON array of strings, masked and bounded. Written with its end, so
  -- NULL while it runs, and when it noted nothing.
  notes TEXT,
  -- The screenshot after it: NULL (none, or its job was dropped), 'stored' (a JPEG on disk,
  -- screenshot_bytes long), 'withheld' (its tab was private) or 'expired' (the disk cap
  -- deleted it).
  screenshot TEXT,
  screenshot_bytes INTEGER,
  PRIMARY KEY (run_id, seq)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS operations_by_start ON operations(started_at, run_id, seq);
CREATE INDEX IF NOT EXISTS operations_by_tab ON operations(target_tab_id, started_at, run_id, seq)
  WHERE target_tab_id IS NOT NULL;
-- The operations still open on a tab, which a lock's settle polls (`open_writes`). Partial, so it
-- holds only open operations and stays small however long the tab's history grows.
CREATE INDEX IF NOT EXISTS operations_open_by_tab ON operations(target_tab_id)
  WHERE outcome IS NULL AND target_tab_id IS NOT NULL;
-- The screenshots on disk, by run. The disk cap sums their sizes from this index alone.
CREATE INDEX IF NOT EXISTS operations_stored_shots ON operations(run_id, screenshot_bytes)
  WHERE screenshot = 'stored';
-- Which runs acted on which tab: a row per run and target tab, written with the operation. The
-- primary key answers runs on this tab; the second index reads a run's tabs and sweeps them.
CREATE TABLE IF NOT EXISTS run_tabs (
  tab_id INTEGER NOT NULL,
  run_id TEXT NOT NULL,
  PRIMARY KEY (tab_id, run_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS run_tabs_by_run ON run_tabs(run_id);
-- Sticky pixel privacy, one row per private tab. Privacy deletes a row when its tab closes;
-- the retention sweep never does, because a private tab can outlive any run on it.
CREATE TABLE IF NOT EXISTS private_tabs (
  tab_id INTEGER PRIMARY KEY,
  since INTEGER NOT NULL,
  reason TEXT NOT NULL
);
