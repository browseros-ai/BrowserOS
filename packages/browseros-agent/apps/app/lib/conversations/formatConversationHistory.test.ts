import { expect, test } from 'bun:test'
import type { UIMessage } from 'ai'
import {
  type ConversationMessage,
  formatConversationHistory,
} from './formatConversationHistory'

const message = (
  id: string,
  role: UIMessage['role'],
  text: string,
): UIMessage => ({
  id,
  role,
  parts: [{ type: 'text', text }],
})

const firstQuestion = message('user-1', 'user', 'First question')
const firstAnswer = message('assistant-1', 'assistant', 'First answer')
const secondQuestion = message('user-2', 'user', 'Second question')
const firstExchange: ConversationMessage[] = [
  { role: 'user', content: 'First question' },
  { role: 'assistant', content: 'First answer' },
]

test('omits replay on the first incognito turn', () => {
  expect(
    formatConversationHistory([firstQuestion], firstQuestion.id),
  ).toBeUndefined()
})

test('replays only the first exchange on the second incognito turn', () => {
  expect(
    formatConversationHistory(
      [firstQuestion, firstAnswer, secondQuestion],
      secondQuestion.id,
    ),
  ).toEqual(firstExchange)
})

test('omits the question and its partial reply when retrying the same turn', () => {
  const partialReply = message('assistant-2', 'assistant', 'Partial reply')

  expect(
    formatConversationHistory(
      [firstQuestion, firstAnswer, secondQuestion, partialReply],
      secondQuestion.id,
    ),
  ).toEqual(firstExchange)
})
