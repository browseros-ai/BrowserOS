"use client";

import { useCallback, useLayoutEffect, useRef, useState } from "react";

/**
 * Where the keyboard or the pointer last moved to: the row's id, stamped with
 * the query it was placed under.
 */
type RowCursor = { id: string; query: string };

/** The cursor's row, or -1 once the query has moved on or the row has left. */
function indexOfCursor(
  rows: readonly { id: string }[],
  query: string,
  cursor: RowCursor | null,
) {
  if (cursor === null || cursor.query !== query) return -1;
  return rows.findIndex((row) => row.id === cursor.id);
}

/**
 * The highlighted row of a list whose rows can change under it.
 *
 * Which row is highlighted is resolved during render, never in a passive
 * effect: a passive effect runs after the commit, so a list that had just
 * changed would carry an `aria-activedescendant` naming a row that has left it,
 * and a key pressed in that window would commit the wrong row or nothing at
 * all.
 *
 * The cursor holds the row's id, not its position. A position alone cannot tell
 * a list that shrank from one that swapped its rows for a different set of the
 * same length, and the second case is the one that silently hands Enter to a
 * row the user never chose. A cursor whose row has left the list returns the
 * highlight to the first row rather than to the nearest surviving one: the row
 * the user aimed at is gone, and the first row is where a new query already
 * puts the highlight.
 *
 * Pass the query the list is filtered by. The cursor is stamped with it and
 * dropped when it changes, so a caller cannot forget to clear it — including a
 * caller whose query arrives as a prop and so never runs its own handler. Rows
 * that come back under a query that has moved on cannot revive it either.
 * `moveTo(null)` is for deliberate resets, such as reopening the list.
 *
 * Both callbacks keep one identity for the life of the component, and read the
 * rows and the query through a ref to do it. A caller will put them in an
 * effect's dependencies — the exhaustive-deps rule makes it — and a `moveTo`
 * rebuilt on every keystroke would re-run that effect on every keystroke.
 */
export function useRowCursor(rows: readonly { id: string }[], query: string) {
  const [cursor, setCursor] = useState<RowCursor | null>(null);
  // Written after commit, not during render: a render React discards or has not
  // finished still runs the component body, and an event handler that read this
  // in that window would stamp the cursor with a query the committed tree does
  // not have.
  const latest = useRef({ rows, query });
  useLayoutEffect(() => {
    latest.current = { rows, query };
  });

  const cursorRow = indexOfCursor(rows, query, cursor);
  // Cleared rather than ignored: React re-runs this render with the cursor
  // already gone, so rows that come back cannot revive a highlight the user has
  // stopped aiming at.
  if (cursor !== null && cursorRow < 0) setCursor(null);

  const moveTo = useCallback(
    (id: string | null) =>
      setCursor(id === null ? null : { id, query: latest.current.query }),
    [],
  );

  const moveActive = useCallback((direction: 1 | -1) => {
    const { rows: live, query: liveQuery } = latest.current;
    const last = live.length - 1;
    if (last < 0) return;
    // Steps from the row the cursor is really on, inside the update, so that
    // two keys landing in one batch move two rows rather than one.
    setCursor((current) => {
      const at = Math.max(indexOfCursor(live, liveQuery, current), 0);
      const next = Math.min(Math.max(at + direction, 0), last);
      return { id: live[next].id, query: liveQuery };
    });
  }, []);

  return { activeIndex: cursorRow < 0 ? 0 : cursorRow, moveTo, moveActive };
}
