/**
 * The control: an ordinary page with no mark and no fact, through every format, so each other
 * gap's cases show what its rule adds to this.
 */
import { calls, chain, cover, everyFormat, type Gap, tree } from './model'

const shop = tree([
  {
    role: 'banner',
    ref: 'e1',
    children: [
      {
        role: 'link',
        name: 'Home',
        ref: 'e2',
        url: 'https://shop.example/',
        cursor: 'pointer',
      },
      {
        role: 'link',
        name: 'Cart (0)',
        ref: 'e3',
        url: 'https://shop.example/cart',
        cursor: 'pointer',
      },
    ],
  },
  {
    role: 'main',
    ref: 'e4',
    children: [
      { role: 'heading', name: 'Mugs', level: 1, ref: 'e5' },
      {
        role: 'paragraph',
        ref: 'e6',
        children: ['Made to order. Free shipping over $40.'],
      },
      {
        role: 'list',
        ref: 'e7',
        children: [
          {
            role: 'listitem',
            ref: 'e8',
            children: [
              {
                role: 'link',
                name: 'Blue mug',
                ref: 'e9',
                url: 'https://shop.example/mugs/blue',
                cursor: 'pointer',
              },
              '$18',
              {
                role: 'button',
                name: 'Add Blue mug to cart',
                ref: 'e10',
                cursor: 'pointer',
              },
            ],
          },
        ],
      },
    ],
  },
])

export const control: Gap = {
  snapshots: {
    'control/shop': shop,
    'control/chain': chain({
      depth: 3,
      link: { role: 'region' },
      leaf: { role: 'button', name: 'Deep', ref: 'e1' },
    }),
  },
  cases: [
    ...calls('control/shop', { ...everyFormat(), cover: cover() }),
    ...calls('control/chain', everyFormat()),
    { name: 'control/cover-null', snapshot: null, call: cover() },
  ],
}
