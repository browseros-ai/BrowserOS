-- Data as a build that knew only m0001_trace_tables wrote it: one settled run of session task-42.
-- Call 1 read a heading; call 2 filled a password, its run timed out under it after the input
-- went out, and its delivery settled as unknown. Frozen: never edit, add a fixture instead.
INSERT INTO trace_sessions (session_key, client, first_run_id, last_run_id, runs, last_activity_ms,
  withheld)
VALUES ('task-42', 'claude-code', '01JC8Z3K4R0000000000000001', '01JC8Z3K4R0000000000000001', 1,
  1730900030500, 0);

INSERT INTO trace_runs (run_id, session_key, client, start_page, timeout_ms, code_bytes, script,
  script_truncated, started_at_ms, status, end_cause, error_name, error_message, value_json,
  value_truncated, ended_at_ms, duration_us, calls_made, calls_over_cap, over_cap_delivery,
  unreadable_calls, getters_over_cap, settled_at_ms, settled_by, detail_bytes, events_stubbed,
  events_dropped, incomplete)
VALUES ('01JC8Z3K4R0000000000000001', 'task-42', 'claude-code', 'TARGET-1', 30000, 61,
  'await page.getByLabel("Password").fill("[redacted]")', 0, 1730900000000, 'timeout', 'deadline',
  'TimeoutError', 'Timeout 30000ms exceeded.', NULL, 0, 1730900030500, 30500000, 2, 0, NULL, 0, 0,
  1730900030600, 'runtime', 180, 1, 0, 0);

INSERT INTO trace_calls (run_id, call_no, parent_no, object, page_id, tab_id, method, title, selector,
  params_json, params_truncated, timeout_ms, loc_line, loc_column, started_at_ms, offset_us,
  duration_us, input_offset_us, state, error_name, error_message, unfinished_reason, delivery,
  delivery_settled, log_json, detail)
VALUES
  ('01JC8Z3K4R0000000000000001', 1, NULL, 'frame@TARGET-1', 'TARGET-1', 7, 'textContent',
   'Get text content', 'internal:role=heading', '{"selector":"internal:role=heading"}', 0, 30000.0,
   1, 7, 1730900000010, 10000, 2000, NULL, 'ok', NULL, NULL, NULL, NULL, 0, NULL, 'full'),
  ('01JC8Z3K4R0000000000000001', 2, NULL, 'frame@TARGET-1', 'TARGET-1', 7, 'fill', 'Fill "{value}"',
   'internal:label="Password"i',
   '{"selector":"internal:label=\"Password\"i","value":"[redacted 9]"}', 0, 0.0, 2, 7,
   1730900000020, 20000, 30480000, 25000, 'unfinished', NULL, NULL, 'timeout', 'unknown', 1,
   '["waiting for getByLabel(''Password'')","  - fill(\"[redacted]\")"]', 'full');

INSERT INTO trace_getters (run_id, getter_no, object, field, page_id, tab_id, outcome, summary_json,
  error_name, error_message, offset_us, duration_us, detail)
VALUES ('01JC8Z3K4R0000000000000001', 1, 'page@TARGET-1', 'url', 'TARGET-1', 7, 'ok',
  '"https://example.com/login?next=[redacted]"', NULL, NULL, 15000, 800, 'full');

INSERT INTO trace_pages (run_id, page_id, tab_id, created, by_call, opened_offset_us,
  closed_offset_us)
VALUES ('01JC8Z3K4R0000000000000001', 'TARGET-1', 7, 0, NULL, 0, NULL);

INSERT INTO trace_logs (run_id, line_no, offset_us, text)
VALUES ('01JC8Z3K4R0000000000000001', 1, 12000, 'filling the password');

INSERT INTO trace_run_ids (run_id, key, value)
VALUES ('01JC8Z3K4R0000000000000001', 'zuzu.task', 't_1');
