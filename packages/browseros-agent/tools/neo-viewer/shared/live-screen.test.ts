import { expect, test } from 'bun:test'
import {
  type DisplayRequest,
  displayQuery,
  parseDisplayQuery,
} from './live-screen'

test('a display request survives the trip through its query', () => {
  const requests: DisplayRequest[] = [
    { mode: 'view' },
    { mode: 'view', followTask: 'claude-code-20261111-101403-3f9a' },
    { mode: 'view', followTask: 'a task with spaces & signs' },
    { mode: 'control' },
    { mode: 'control', release: 'tk_01a1cc70-724b-7683-b242-0f842152aec0' },
  ]
  for (const request of requests) {
    expect(
      parseDisplayQuery(new URLSearchParams(displayQuery(request))),
    ).toEqual(request)
  }
})

test('queries Neo would refuse are refused before they reach it', () => {
  for (const query of [
    '',
    'mode=bogus',
    'mode=view&release=tk_1',
    'mode=control&followTask=t1',
    'mode=view&followTab=3',
    'mode=view&mode=control',
    'mode=view&followTask=',
    'mode=control&release=',
    `mode=view&followTask=${'é'.repeat(65)}`,
  ]) {
    expect(parseDisplayQuery(new URLSearchParams(query))).toHaveProperty(
      'error',
    )
  }
  expect(
    parseDisplayQuery(
      new URLSearchParams(`mode=view&followTask=${'x'.repeat(128)}`),
    ),
  ).toEqual({ mode: 'view', followTask: 'x'.repeat(128) })
})
