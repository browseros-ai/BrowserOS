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

it('keeps a held page current after a reply binds the built-in page to its tab', async () => {
  // As neo.task's reply does once the script already holds a Page for the task's tab: the
  // built-in page becomes the tab's entry, and the object the script holds keeps its facts.
  const open = {
    tab: 101,
    taskId: 't-1',
    url: 'https://a.example/',
    title: 'A',
    closed: false,
  }
  const replies = [
    {
      ok: { $neo: 'page', tab: 101 },
      pages: [open],
      observed: { seq: 1, events: 0 },
    },
    {
      ok: null,
      defaultPage: { tab: 101 },
      pages: [{ ...open, url: 'https://b.example/' }],
      observed: { seq: 2, events: 0 },
    },
    {
      ok: null,
      pages: [{ ...open, url: null, title: null, closed: true }],
      observed: { seq: 3, events: 1 },
    },
  ]
  const runtime = loadPrelude({ answer: () => replies.shift() })
  expect(
    json(
      await runtime.run(`
    const held = await __neo.invoke('test.page', {});
    await __neo.invoke('test.pick', {});
    const bound = { page: [page.tabId, page.url()], held: held.url(),
      entry: context.pages().length === 1 && context.pages()[0] === page };
    await __neo.invoke('test.close', {});
    return { bound, closed: [page.isClosed(), held.isClosed()] };
  `),
    ),
  ).toEqual({
    bound: {
      page: [101, 'https://b.example/'],
      held: 'https://b.example/',
      entry: true,
    },
    closed: [true, true],
  })
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
    answer: () => ({ ok: null, pages: [facts.shift()] }),
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
  const runtime = loadPrelude({ answer: () => results.shift() })
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
