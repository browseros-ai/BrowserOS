import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { detailWith } from '../../server/test-support/neo-records'
import { SNAPSHOT, TRAILED } from '../test-support/replies'
import { FormattedReply, previewText } from './RunList'

const text = (markup: string) =>
  markup
    .replace(/<[^>]+>/g, '\n')
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/\n+/g, '\n')
    .trim()

describe("the plain page's reply", () => {
  it('shows a returned string decoded, with a line saying the model received it as a JSON string', () => {
    const snapshot =
      '- link "Hacker News" [ref=e16]\n- text: "|"\n- text: ```x```'
    const json = JSON.stringify(snapshot)
    const shown = text(
      renderToStaticMarkup(
        <FormattedReply detail={detailWith(1, { text: json, neo: null })} />,
      ),
    )
    expect(shown).toContain(
      `What the model read\nShown decoded; the model received it as a JSON string of ${json.length} characters.`,
    )
    expect(shown).toContain(
      '- link "Hacker News" [ref=e16]\n- text: "|"\n- text: ```x```',
    )
    expect(shown).not.toContain('\\n')
  })

  it('shows run 8, and the same value before an unknown line, decoded with the line under it', () => {
    for (const [reply, trailer] of TRAILED) {
      const shown = text(
        renderToStaticMarkup(
          <FormattedReply detail={detailWith(8, { text: reply, neo: null })} />,
        ),
      )
      expect(shown).toContain('Shown decoded')
      expect(shown).toContain(SNAPSHOT.split('\n').slice(0, 3).join('\n'))
      expect(shown).toContain(trailer)
      expect(shown).not.toContain('\\n- ')
    }
  })

  it('shows any other value as the model read it', () => {
    const shown = text(
      renderToStaticMarkup(
        <FormattedReply
          detail={detailWith(2, { text: '{"a":1}', neo: null })}
        />,
      ),
    )
    expect(shown).toContain('{"a":1}')
    expect(shown).not.toContain('Shown decoded')
  })

  it("decodes a closed row's preview of a returned string, even when the preview cut it short", () => {
    const json = JSON.stringify(
      '- link "Hacker News" [ref=e16]\n- text: "|"\n- link "new"',
    )
    expect(previewText(json)).toBe(
      '- link "Hacker News" [ref=e16]\n- text: "|"\n- link "new"',
    )
    expect(previewText(json.slice(0, 25))).toBe('- link "Hacker News" [')
    expect(previewText(json.slice(0, json.indexOf('\\n') + 1))).toBe(
      '- link "Hacker News" [ref=e16]',
    )
    // Cut right after a whole `\\` escape, and inside a `\u0000` escape.
    const slash = JSON.stringify('path C:\\dir')
    expect(previewText(slash.slice(0, -1))).toBe('path C:\\dir')
    expect(previewText(`${JSON.stringify('a\\')}`.slice(0, -1))).toBe('a\\')
    const nul = JSON.stringify('a\u0000b')
    for (let end = 2; end < nul.length; end++)
      expect(previewText(nul.slice(0, end)).startsWith('a')).toBe(true)
    expect(previewText(nul.slice(0, 4))).toBe('a')
    expect(previewText('{"a":1}')).toBe('{"a":1}')
    expect(previewText('script / no_match: x')).toBe('script / no_match: x')
  })
})
