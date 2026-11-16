// Neo's page identifiers across init facts, reply facts and revived result handles.
import { expect, it } from 'bun:test'
import { json, loadPrelude } from './support/prelude-runtime'

it('keeps both identifiers null until the built-in page opens', async () => {
  const runtime = loadPrelude({
    answer: () => ({
      ok: 'Opened',
      defaultPage: { tab: 101 },
      pages: [{ tab: 101, taskId: 't-auto' }],
    }),
  })
  expect(
    json(
      await runtime.run(`
    const before = [page.tabId, page.taskId];
    await page.title();
    return { before, after: [page.tabId, page.taskId], same: context.pages()[0] === page };
  `),
    ),
  ).toEqual({ before: [null, null], after: [101, 't-auto'], same: true })
  expect(runtime.calls.map((call) => call.name)).toEqual(['page.title'])
})

// A task pick's reply names the tab it bound the run's page to as `pickedPage`, with that
// tab's fact. The init lists every tab, so the tab usually has an object already.
const open = {
  tab: 101,
  taskId: 't-1',
  url: 'https://task.example/',
  title: 'Task',
  closed: false,
}
const picked = {
  ok: null,
  pickedPage: { tab: 101 },
  pages: [open],
  observed: { seq: 2, events: 0 },
}

it('binds the built-in page at a task pick when its tab has not reached the script', async () => {
  const runtime = loadPrelude({
    seed: { page: null, pages: [{ ...open, url: 'about:blank' }] },
    answer: () => picked,
  })
  expect(
    json(
      await runtime.run(`
    await __neo.invoke('neo.task', { id: 't-1' });
    return { page: [page.tabId, page.taskId, page.url()],
      entry: context.pages().length === 1 && context.pages()[0] === page };
  `),
    ),
  ).toEqual({ page: [101, 't-1', 'https://task.example/'], entry: true })
})

it('leaves a pick to the first action when the script holds the tab', async () => {
  // Replacing a held Page as its tab's entry would strand its listeners and bindings, so the
  // built-in page binds on the reply to its own first op, which names the same tab.
  const replies = [
    {
      ok: { $neo: 'page', tab: 101 },
      pages: [open],
      observed: { seq: 1, events: 0 },
    },
    picked,
    {
      ok: 'Task',
      defaultPage: { tab: 101 },
      pages: [open],
      observed: { seq: 3, events: 0 },
    },
  ]
  // Neo's own frame reads (of a page first seen mid-run, and the retry of a tree it could not
  // read) get no tree and leave the scripted replies to the script's calls.
  const runtime = loadPrelude({
    answer: (call) => (call.name === 'page.frames' ? null : replies.shift()),
  })
  expect(
    json(
      await runtime.run(`
    const held = await __neo.invoke('test.page', {});
    await __neo.invoke('neo.task', { id: 't-1' });
    const atPick = { page: page.tabId, entry: context.pages()[0] === held };
    await page.title();
    return { atPick, after: page.tabId, entry: context.pages()[0] === page };
  `),
    ),
  ).toEqual({ atPick: { page: null, entry: true }, after: 101, entry: true })
})

it('counts a tab context.pages() listed as held at a pick', async () => {
  const runtime = loadPrelude({
    seed: { page: null, pages: [open] },
    // The listed tab's frame read at the run's start gets no tree; every other op the pick.
    answer: (call) => (call.name === 'page.frames' ? null : picked),
  })
  expect(
    json(
      await runtime.run(`
    const listed = context.pages();
    await __neo.invoke('neo.task', { id: 't-1' });
    return { page: page.tabId, entry: context.pages()[0] === listed[0] };
  `),
    ),
  ).toEqual({ page: null, entry: true })
})

it('reads task ownership from init facts without changing Page identity', async () => {
  const runtime = loadPrelude({
    seed: {
      page: { $neo: 'page', tab: 101 },
      pages: [
        { tab: 101, taskId: 'research-1234' },
        { tab: 102, taskId: null },
        { tab: 103 },
      ],
    },
  })
  expect(
    json(
      await runtime.run(`return {
    pages: context.pages().map((p) => [p.tabId, p.taskId]),
    same: page === context.pages()[0],
  }`),
    ),
  ).toEqual({
    pages: [
      [101, 'research-1234'],
      [102, null],
      [103, null],
    ],
    same: true,
  })
})

it('retains an omitted task id and clears an explicit null in reply facts', async () => {
  const facts = [
    { tab: 101, taskId: 'research-1234' },
    { tab: 101 },
    { tab: 101, taskId: 7 },
    { tab: 101, taskId: null },
  ]
  const runtime = loadPrelude({
    answer: (call) =>
      call.name === 'page.frames' ? null : { ok: null, pages: [facts.shift()] },
  })
  expect(
    json(
      await runtime.run(`
    const ids = [];
    for (let i = 0; i < 4; i++) {
      await __neo.invoke('test.facts', {});
      ids.push(context.pages()[0].taskId);
    }
    return ids;
  `),
    ),
  ).toEqual(['research-1234', 'research-1234', 'research-1234', null])
})

it('revives result facts with task ids and preserves them on later bare brands', async () => {
  const results = [
    { $neo: 'page', tab: 101, taskId: 'research-1234' },
    { $neo: 'page', tab: 101 },
    { $neo: 'page', tab: 101, taskId: null },
  ]
  const runtime = loadPrelude({
    answer: (call) => (call.name === 'page.frames' ? null : results.shift()),
  })
  expect(
    json(
      await runtime.run(`
    const first = await __neo.invoke('test.page', {});
    const initial = [first.tabId, first.taskId];
    const second = await __neo.invoke('test.page', {});
    const retained = second.taskId;
    const third = await __neo.invoke('test.page', {});
    return { initial, retained, cleared: third.taskId,
      same: first === second && second === third && third === context.pages()[0] };
  `),
    ),
  ).toEqual({
    initial: [101, 'research-1234'],
    retained: 'research-1234',
    cleared: null,
    same: true,
  })
})

it('exposes identifiers as read-only getters', async () => {
  const runtime = loadPrelude()
  expect(
    json(
      await runtime.run(`return ['tabId', 'taskId'].map((key) => {
    const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(page), key);
    try { page[key] = 'changed'; } catch (error) {
      return [typeof descriptor.get, descriptor.set === undefined, error instanceof TypeError];
    }
    return false;
  })`),
    ),
  ).toEqual([
    ['function', true, true],
    ['function', true, true],
  ])
})
