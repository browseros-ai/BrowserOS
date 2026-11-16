import { describe, expect, it } from 'bun:test'
import { type Comment, tokenizer } from 'acorn'
import { FORMAT_WIDTH, firstStatement, formatCode } from './code-format'

const OPTIONS = {
  ecmaVersion: 'latest',
  sourceType: 'script',
  allowAwaitOutsideFunction: true,
  allowReturnOutsideFunction: true,
} as const

/** Every token's text and every comment, in order: what whitespace-only formatting must keep. */
function tokensOf(code: string): string[] {
  const comments: Comment[] = []
  const texts = [...tokenizer(code, { ...OPTIONS, onComment: comments })].map(
    (token) => code.slice(token.start, token.end),
  )
  return [...texts, ...comments.map((comment) => `/*${comment.value}*/`)]
}

// The owner's screenshot: what agents send, minified on one line.
const OWNER =
  "const p=await context.newPage();await p.goto(\"https://example.com\");const info=await p.evaluate(()=>({title:document.title,links:[...document.querySelectorAll('a')].map(a=>a.href).slice(0,5),h1:document.querySelector('h1')?.textContent}));return info;"

// Minified scripts as agents sent them through the viewer.
const SCRIPTS = [
  OWNER,
  "await page.goto('https://en.wikipedia.org/wiki/Grace_Hopper');return await page.title();",
  "const box=page.locator('input[name=\"search\"]:visible').first();await box.fill('COBOL');await box.press('Enter');await page.waitForURL(/COBOL/);await page.waitForLoadState('domcontentloaded');return{title:await page.title(),url:page.url()};",
  // ; and braces inside strings, a template literal and a regex.
  // biome-ignore lint/suspicious/noTemplateCurlyInString: the script holds a template literal.
  "const s='a;b{c}';const t=`x;${s}{`;const r=/;\\{[}]/g;return[s.split(';'),t,r.test('{')];",
  // for(;;) headers keep their semicolons on one line.
  'for(let i=0;i<3;i++){for(;;){break}}let n=0;while(n<2){n++}do{n--}while(n>0);',
  // Nested callbacks, a block body inside an expression body.
  "const rows=await page.$$eval('tr',trs=>trs.map(tr=>{const cells=[...tr.querySelectorAll('td')];return cells.map(td=>td.textContent.trim())}));return rows.filter(r=>r.length>0);",
  'const [download]=await Promise.all([page.waitForEvent(\'download\'),page.getByText("Download October statement").click()]);return{name:download.suggestedFilename(),path:await download.path()};',
  // Comments, line and block.
  "// open the form\nawait page.goto('http://127.0.0.1:8800/form.html');/* then read */return await page.title();",
  'try{await page.click("#a")}catch(e){console.log("no;a",e.message)}finally{await page.screenshot()}if(x){a()}else if(y){b()}else{c()}',
  'switch(mode){case"a":await one();break;default:await two()}class A{m(){return 1}}const o={a:1,...rest,[k]:v,f(){return 2}};',
]

describe('formatting a run script for reading', () => {
  it('changes whitespace only: every token and comment is kept, in order', () => {
    for (const code of SCRIPTS) {
      const { text } = formatCode(code)
      expect(tokensOf(text)).toEqual(tokensOf(code))
      expect(text.replace(/\s+/g, '')).toBe(code.replace(/\s+/g, ''))
    }
  })

  it("lays out the owner's script one statement per line, its callback indented under the call", () => {
    expect(formatCode(OWNER).text).toBe(
      [
        'const p = await context.newPage();',
        'await p.goto("https://example.com");',
        'const info = await p.evaluate(() =>',
        '  ({',
        '    title: document.title,',
        "    links: [...document.querySelectorAll('a')].map(a => a.href).slice(0, 5),",
        "    h1: document.querySelector('h1')?.textContent",
        '  })',
        ');',
        'return info;',
      ].join('\n'),
    )
  })

  it("maps each method's site (Neo's line and column, at the method name) to its display line", () => {
    const formatted = formatCode(OWNER)
    const site = (name: string) => ({
      line: 1,
      column: OWNER.indexOf(name) + 1,
    })
    expect(formatted.lineOfSite(site('newPage'))).toBe(1)
    expect(formatted.lineOfSite(site('goto'))).toBe(2)
    expect(formatted.lineOfSite(site('evaluate'))).toBe(3)
    // Methods inside the callback land on the lines that now hold them.
    expect(formatted.lineOfSite(site('querySelectorAll'))).toBe(6)
    expect(formatted.lineOfSite({ line: 2, column: 1 })).toBeNull()
  })

  it('maps sites on later lines of a multi-line script too', () => {
    const code = "const a=1;\nawait page.goto('x');await page.click('#b');"
    const formatted = formatCode(code)
    expect(formatted.lineOfSite({ line: 2, column: 12 })).toBe(2)
    expect(
      formatted.lineOfSite({
        line: 2,
        column: (code.split('\n')[1]?.indexOf('click') as number) + 1,
      }),
    ).toBe(3)
  })

  it('keeps no line wider than the column when there is something to break', () => {
    for (const code of SCRIPTS) {
      for (const line of formatCode(code).lines) {
        // A single string or a short chain cannot be broken further; none of these has one.
        expect(line.length).toBeLessThanOrEqual(FORMAT_WIDTH)
      }
    }
  })

  it('leaves a script the agent already formatted as it was', () => {
    const formatted = [
      '// read the title and the first heading',
      'const title = await page.title();',
      "const heading = await page.locator('h1').first().textContent();",
      'if (!title) {',
      "  throw new Error('no title');",
      '}',
      'return { title, heading };',
    ].join('\n')
    expect(formatCode(formatted).text).toBe(formatted)
    const fourSpaces = 'if (a) {\n    await b();\n    await c();\n}'
    expect(formatCode(fourSpaces).text).toBe(fourSpaces)
  })

  it('shows code that does not parse as sent, line for line', () => {
    const broken = 'const x = ;\nawait page.goto("a")'
    const formatted = formatCode(broken)
    expect(formatted.text).toBe(broken)
    expect(formatted.lineOfSite({ line: 2, column: 12 })).toBe(2)
  })

  it('never breaks inside a template literal or a for header, even for width', () => {
    const long = Array.from({ length: 30 }, (_, i) => `'item${i}'`).join(',')
    const template = `const t=\`\${[${long}].join(',')}\`;`
    expect(formatCode(template).lines).toHaveLength(1)
    const header = `for(const x of [${long}]){await f(x)}`
    expect(formatCode(header).lines[0]).toStartWith('for (const x of [')
    expect(formatCode(header).lines[0]).toEndWith('{')
  })

  it("breaks a callback's body in any argument position when its line is too wide", () => {
    const code =
      "const text=await page.evaluate(sel=>document.querySelector(sel).textContent.trim().toUpperCase(),'#main h1');"
    const { lines, text } = formatCode(code)
    expect(tokensOf(text)).toEqual(tokensOf(code))
    expect(lines.every((line) => line.length <= FORMAT_WIDTH)).toBe(true)
  })

  it('breaks a construct on any too-wide line it reaches, not only its first', () => {
    // An object the agent opened on its own line, with members sharing one long line.
    const code = `const o = {\n  a: '${'x'.repeat(40)}', b: '${'y'.repeat(40)}'\n};`
    const { lines } = formatCode(code)
    expect(lines.every((line) => line.length <= FORMAT_WIDTH)).toBe(true)
  })

  it('breaks every too-wide line, however many there are', () => {
    const row = (i: number) =>
      `const a${i}=[${Array.from({ length: 12 }, (_, j) => `'v${j}'`).join(',')}];`
    const code = Array.from({ length: 220 }, (_, i) => row(i)).join('')
    expect(
      formatCode(code).lines.every((line) => line.length <= FORMAT_WIDTH),
    ).toBe(true)
  })

  it('gives a closed row the first statement, on one line', () => {
    expect(firstStatement(OWNER)).toBe('const p = await context.newPage();')
    expect(firstStatement("// note\nawait page.goto('x');return 1")).toBe(
      "await page.goto('x');",
    )
    expect(firstStatement('const x = ;\nmore')).toBe('const x = ;')
    // Comments stay as sent; a token that spans lines is cut where its first line ends.
    expect(firstStatement('const/*keep*/x=1;return x;')).toBe(
      'const/*keep*/x = 1;',
    )
    expect(firstStatement('const t=`a\nb`;return t;')).toBe('const t = `a…')
  })
})
