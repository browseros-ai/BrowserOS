/** Desktop MCP contracts for the unchanged SDK's Playwright realm. Scripts
 * check browser effects through Playwright; host assertions check the wire reply.
 * The runner closes any tabs left by a script, including one that throws. */
import { expect } from 'bun:test'
import { join } from 'node:path'
import type { ContractCase } from './cases'
import { expectError } from './helpers'
import { runValue } from './sdk-helpers'

export const playwrightRunCases: ContractCase[] = [
  {
    name: 'playwright run: navigate and read a page',
    smoke: true,
    async run(ctx) {
      const url = ctx.fixture('/links.html')
      expect(
        await runValue(
          ctx,
          `await page.goto(${JSON.stringify(url)});
           return { title: await page.title(), url: page.url(),
             heading: await page.getByRole('heading', { level: 1 }).innerText() };`,
        ),
      ).toEqual({ title: 'Links fixture', url, heading: 'Links fixture' })
    },
  },
  {
    name: 'playwright run: ariaSnapshot refs drive getByRef click',
    smoke: true,
    async run(ctx) {
      expect(
        await runValue(
          ctx,
          String.raw`await page.goto(${JSON.stringify(ctx.fixture('/form.html'))});
            const snapshot = await page.ariaSnapshot();
            const line = snapshot.split('\n').find(line => line.includes('button "Apply"'));
            const ref = line?.match(/\[ref=([^\]]+)\]/)?.[1];
            if (!ref) throw new Error('Apply button has no snapshot ref: ' + snapshot);
            await page.getByRef(ref).click();
            return await page.locator('#result').innerText();`,
        ),
      ).toBe('applied')
    },
  },
  {
    name: 'playwright run: fill a field and read its submitted value',
    smoke: true,
    async run(ctx) {
      expect(
        await runValue(
          ctx,
          `await page.goto(${JSON.stringify(ctx.fixture('/form.html'))});
           const name = page.getByRole('textbox', { name: 'Name', exact: true });
           await name.fill('Ada Lovelace');
           await page.getByRole('button', { name: 'Submit', exact: true }).click();
           return { value: await name.inputValue(),
             result: await page.locator('#result').innerText() };`,
        ),
      ).toEqual({
        value: 'Ada Lovelace',
        result:
          'submitted name=Ada Lovelace nickname=prefilled-nick size=small',
      })
    },
  },
  {
    name: 'playwright run: context.newPage opens distinct readable tabs',
    smoke: true,
    async run(ctx) {
      expect(
        await runValue(
          ctx,
          `const first = await context.newPage();
           await first.goto(${JSON.stringify(ctx.fixture('/links.html'))});
           const second = await context.newPage();
           await second.goto(${JSON.stringify(ctx.fixture('/form.html'))});
           return { titles: [await first.title(), await second.title()],
             distinct: first.tabId !== second.tabId,
             listed: context.pages().some(p => p.tabId === first.tabId)
               && context.pages().some(p => p.tabId === second.tabId) };`,
        ),
      ).toEqual({
        titles: ['Links fixture', 'Form fixture'],
        distinct: true,
        listed: true,
      })
    },
  },
  {
    name: 'playwright run: upload a Buffer payload without a host path',
    smoke: true,
    async run(ctx) {
      expect(
        await runValue(
          ctx,
          `await page.goto(${JSON.stringify(ctx.fixture('/upload.html'))});
           await page.locator('#single').setInputFiles({ name: 'payload.txt',
             mimeType: 'text/plain', buffer: Buffer.from('contract payload') });
           return { status: await page.locator('#single-names').innerText(),
             file: await page.locator('#single').evaluate(async input => {
               const file = input.files[0];
               return { name: file.name, type: file.type, text: await file.text() };
             }) };`,
        ),
      ).toEqual({
        status: 'payload.txt',
        file: {
          name: 'payload.txt',
          type: 'text/plain',
          text: 'contract payload',
        },
      })
    },
  },
  {
    name: 'playwright run: download.saveAs writes the downloaded bytes',
    smoke: true,
    async run(ctx) {
      const destination = join(ctx.scratchDir, 'run-report.txt')
      expect(
        await runValue(
          ctx,
          `await page.goto(${JSON.stringify(ctx.fixture('/links.html'))});
           const waiting = page.waitForEvent('download', { timeout: 10000 });
           await page.getByRole('link', { name: 'Download report', exact: true }).click();
           const download = await waiting;
           await download.saveAs(${JSON.stringify(destination)});
           return { name: download.suggestedFilename(), failure: await download.failure() };`,
        ),
      ).toEqual({ name: 'report.txt', failure: null })
      expect(await Bun.file(destination).text()).toBe(
        'fixture report contents\n',
      )
    },
  },
  {
    name: 'playwright run: expect failure includes expected and received values',
    smoke: true,
    async run(ctx) {
      const text = expectError(
        await ctx.mcp.callTool('run', {
          code: "expect('contract actual').toBe('contract expected')",
        }),
        'failing Playwright expect',
      )
      expect(text).toContain('contract expected')
      expect(text).toContain('contract actual')
      expect(text).toContain('toBe')
    },
  },
  {
    name: 'playwright run: explicit tasks retain their page across runs',
    async run(ctx) {
      const taskId = await runValue(
        ctx,
        `const task = await neo.createTask('contract task persistence');
         await page.goto(${JSON.stringify(ctx.fixture('/form.html'))});
         await page.getByRole('textbox', { name: 'Name', exact: true }).fill('persistent field');
         return task.id;`,
      )
      if (typeof taskId !== 'string') throw new Error('run omitted the task id')
      try {
        expect(
          await runValue(
            ctx,
            `const task = await neo.task(${JSON.stringify(taskId)});
             return { url: page.url(), value: await page.getByRole('textbox',
               { name: 'Name', exact: true }).inputValue(), pages: task.pages().length };`,
          ),
        ).toEqual({
          url: ctx.fixture('/form.html'),
          value: 'persistent field',
          pages: 1,
        })
      } finally {
        await runValue(
          ctx,
          `await (await neo.task(${JSON.stringify(taskId)})).close(); return null;`,
        )
      }
    },
  },
  {
    name: 'playwright run: screenshot resolves to PNG bytes',
    async run(ctx) {
      expect(
        await runValue(
          ctx,
          `await page.goto(${JSON.stringify(ctx.fixture('/links.html'))});
           const bytes = await page.screenshot();
           return { prefix: bytes.toString('hex', 0, 8), hasBody: bytes.length > 8 };`,
        ),
      ).toEqual({ prefix: '89504e470d0a1a0a', hasBody: true })
    },
  },
  {
    name: 'playwright run: pdf resolves to PDF bytes',
    async run(ctx) {
      expect(
        await runValue(
          ctx,
          `await page.goto(${JSON.stringify(ctx.fixture('/links.html'))});
           const bytes = await page.pdf();
           return { prefix: bytes.toString('utf8', 0, 5), hasBody: bytes.length > 5 };`,
        ),
      ).toEqual({ prefix: '%PDF-', hasBody: true })
    },
  },
  {
    name: 'playwright run: file navigation is refused',
    async run(ctx) {
      const text = expectError(
        await ctx.mcp.callTool('run', {
          code: "await page.goto('file:///tmp/claw-contract-refused.html')",
        }),
        'file navigation',
      )
      expect(text).toContain('file:')
      expect(text).toContain('cannot open file: pages')
    },
  },
  {
    name: 'playwright run: uploads outside files.readDirs are refused',
    async run(ctx) {
      const outside = join(ctx.scratchDir, 'outside-upload.txt')
      await Bun.write(outside, 'must not reach the page')
      const text = expectError(
        await ctx.mcp.callTool('run', {
          code: `await page.goto(${JSON.stringify(ctx.fixture('/upload.html'))});
            await page.locator('#single').setInputFiles(${JSON.stringify(outside)});`,
        }),
        'upload outside allowed folders',
      )
      expect(text).toContain('outside files.readDirs')
      expect(text).toContain('payload')
    },
  },
]
