/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

const ADJECTIVES = [
  'agile',
  'amber',
  'bold',
  'breezy',
  'bright',
  'brisk',
  'calm',
  'clever',
  'cozy',
  'curious',
  'dapper',
  'eager',
  'fancy',
  'fleet',
  'fuzzy',
  'gentle',
  'glad',
  'golden',
  'happy',
  'jolly',
  'keen',
  'kind',
  'lively',
  'lucky',
  'merry',
  'mighty',
  'nimble',
  'peppy',
  'plucky',
  'proud',
  'quick',
  'quiet',
  'radiant',
  'rapid',
  'ready',
  'silky',
  'snappy',
  'spry',
  'steady',
  'sunny',
  'swift',
  'tidy',
  'vivid',
  'warm',
  'witty',
  'zesty',
] as const

const ANIMALS = [
  'alpaca',
  'badger',
  'beaver',
  'bison',
  'bobcat',
  'capybara',
  'caribou',
  'cheetah',
  'corgi',
  'dolphin',
  'falcon',
  'ferret',
  'finch',
  'fox',
  'gecko',
  'heron',
  'ibis',
  'jaguar',
  'koala',
  'lemur',
  'leopard',
  'lynx',
  'marten',
  'moose',
  'narwhal',
  'ocelot',
  'orca',
  'otter',
  'owl',
  'panda',
  'parrot',
  'penguin',
  'puffin',
  'quokka',
  'rabbit',
  'raven',
  'seal',
  'sparrow',
  'stoat',
  'tamarin',
  'tiger',
  'toucan',
  'turtle',
  'walrus',
  'weasel',
  'wombat',
] as const

const REDRAW_ATTEMPTS = 5
const MAX_SUFFIX = 999

export interface GenerateFunNameOptions {
  random?: () => number
  isAvailable?: (candidate: string) => boolean
}

function pick(words: readonly string[], random: () => number): string {
  const draw = Math.max(0, Math.min(random(), 1 - Number.EPSILON))
  return words[Math.floor(draw * words.length)] as string
}

/** Draws an available docker-style name, suffixing after repeated collisions. */
export function generateFunName(options: GenerateFunNameOptions = {}): string {
  const random = options.random ?? Math.random
  const isAvailable = options.isAvailable ?? (() => true)
  let candidate = ''

  for (let attempt = 0; attempt < REDRAW_ATTEMPTS; attempt++) {
    candidate = `${pick(ADJECTIVES, random)}-${pick(ANIMALS, random)}`
    if (isAvailable(candidate)) return candidate
  }

  for (let suffix = 2; suffix <= MAX_SUFFIX; suffix++) {
    const suffixed = `${candidate}-${suffix}`
    if (isAvailable(suffixed)) return suffixed
  }
  throw new Error('unable to mint a unique session name')
}
