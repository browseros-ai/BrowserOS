import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { detailWith } from '../../server/test-support/neo-records'
import { FormattedReply } from './RunList'

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
      `Shown decoded; the model received it as a JSON string of ${json.length} characters.`,
    )
    expect(shown).toContain(
      '- link "Hacker News" [ref=e16]\n- text: "|"\n- text: ```x```',
    )
    expect(shown).not.toContain('\\n')
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
})
