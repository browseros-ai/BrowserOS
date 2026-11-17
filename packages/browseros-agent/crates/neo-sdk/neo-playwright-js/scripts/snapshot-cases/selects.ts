/**
 * Native selects: their options as the field's value and choices, and the long-select rule.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import { calls, full, type Gap, page, tree } from './model'

/** `count` ref-less options named `Size 1`, `Size 2`, ..., those at `selected` selected. */
function options(count: number, selected: number[] = []): AriaNodeJSON[] {
  return Array.from({ length: count }, (_, index) => ({
    role: 'option',
    name: `Size ${index + 1}`,
    ...(selected.includes(index) ? { selected: true } : {}),
  }))
}

const both = {
  'page-text': page({ text: true }),
  'page-notext': page({ text: false }),
}

export const selects: Gap = {
  snapshots: {
    // 13 options, one past the limit: without text only the selected one, then how many more.
    'selects/thirteen': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: options(13, [3]),
      },
    ]),
    // The "more options" line past a cut is one more line, and not one with a ref.
    'selects/thirteen-cut': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: options(13, [3]),
      },
    ]),
    // 12 options, at the limit: every one is listed without text too.
    'selects/twelve': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: options(12, [3]),
      },
    ]),
    // A long select with nothing selected: without text, only the count of options.
    'selects/none-selected': tree([
      { role: 'combobox', name: 'Size', ref: 'e1', children: options(13) },
    ]),
    // A listbox (a select that shows several rows) with two selected: both, then the rest.
    'selects/listbox': tree([
      {
        role: 'listbox',
        name: 'Sizes',
        ref: 'e1',
        children: options(14, [0, 9]),
      },
    ]),
    // Option groups print no line: their options are the select's, counted across groups.
    'selects/groups': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: [
          { role: 'option', name: 'Any size' },
          {
            role: 'group',
            name: 'Small',
            children: options(7).map((option) => ({
              ...option,
              name: `Small ${option.name}`,
            })),
          },
          'or',
          {
            role: 'group',
            name: 'Large',
            children: options(6, [4]).map((option) => ({
              ...option,
              name: `Large ${option.name}`,
            })),
          },
        ],
      },
    ]),
    // An option with a ref is a control, not a choice: its own line, at the select's level.
    'selects/ref-option': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: [
          ...options(3, [0]),
          { role: 'option', name: 'Custom size', ref: 'e2' },
        ],
      },
    ]),
    // A listbox whose options all have refs has no choices: one line, its options after it.
    'selects/all-ref-options': tree([
      {
        role: 'listbox',
        name: 'Sizes',
        ref: 'e1',
        children: [
          { role: 'option', name: 'Small', ref: 'e2', selected: true },
          { role: 'option', name: 'Large', ref: 'e3' },
        ],
      },
    ]),
    // A clickable group and an option inside a wrapper are not collected, and print nowhere.
    'selects/not-collected': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: [
          { role: 'option', name: 'Any size', selected: true },
          {
            role: 'group',
            name: 'Small',
            ref: 'e2',
            cursor: 'pointer',
            children: [{ role: 'option', name: 'Small one' }],
          },
          {
            role: 'generic',
            children: [{ role: 'option', name: 'Wrapped one' }],
          },
        ],
      },
    ]),
    // A select's own value does not print beside its choices: its selected option says it.
    'selects/value-text': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        text: 'Size 2',
        children: options(3, [1]),
      },
    ]),
    // A select without a ref still lists its choices, and the long rule still holds.
    'selects/no-ref': tree([
      { role: 'combobox', name: 'Size', children: options(13, [12]) },
    ]),
    // An option's line keeps its states and its own text, never a ref, cursor or url.
    'selects/option-lines': tree([
      {
        role: 'combobox',
        name: 'Size',
        ref: 'e1',
        children: [
          { role: 'option', name: 'S', text: 'Small', selected: true },
          { role: 'option', name: 'M', disabled: true },
          {
            role: 'option',
            name: 'L',
            cursor: 'pointer',
            url: 'https://shop.example/size/l',
          },
        ],
      },
    ]),
  },
  cases: [
    ...calls('selects/thirteen', { full: full(), ...both }),
    ...calls('selects/thirteen-cut', {
      'page-notext-max7': page({ text: false, maxTokens: 7 }),
    }),
    ...calls('selects/twelve', { 'page-notext': page({ text: false }) }),
    ...calls('selects/none-selected', { 'page-notext': page({ text: false }) }),
    ...calls('selects/listbox', { 'page-notext': page({ text: false }) }),
    ...calls('selects/groups', both),
    ...calls('selects/ref-option', { 'page-notext': page({ text: false }) }),
    ...calls('selects/all-ref-options', {
      'page-notext': page({ text: false }),
    }),
    ...calls('selects/not-collected', both),
    ...calls('selects/value-text', { full: full(), ...both }),
    ...calls('selects/no-ref', { 'page-notext': page({ text: false }) }),
    ...calls('selects/option-lines', {
      'page-notext-urls': page({ text: false, urls: true }),
    }),
  ],
}
