/** The body of ONE native run. Reflection invokes actual SDK objects through the usual bridge. */
import { codecFactory } from './codec.mjs'

export function nativeLoop(codecFactory, epoch) {
  class ProtocolError extends Error {}
  const objects = new Map()
  const reverse = new WeakMap()
  const seeds = [
    page,
    context,
    page.mainFrame(),
    page.locator('__neo_capability_probe__'),
  ]
  const prototypes = new Map(
    seeds.map((object) => [
      Object.getPrototypeOf(object),
      object.constructor.name,
    ]),
  )
  const identify = (value) => {
    const family = prototypes.get(Object.getPrototypeOf(value))
    if (!family) return undefined
    if (!reverse.has(value)) {
      const ref = {
        epoch,
        id: objects.size + 1,
        family,
      }
      reverse.set(value, ref)
      objects.set(ref.id, value)
    }
    return reverse.get(value)
  }
  const resolve = (ref) => {
    const value = ref?.epoch === epoch ? objects.get(ref.id) : undefined
    if (!value || reverse.get(value).family !== ref.family)
      throw new ProtocolError('Foreign or expired native object')
    return value
  }
  const codec = codecFactory({ identify, resolve })
  const reply = (frame) => __neo_test_host.reply(JSON.stringify(frame))
  const error = (thrown) => ({
    error: codec.encode({
      name: String(thrown?.name || 'Error'),
      message: String(thrown?.message ?? thrown),
    }),
    errorStack: codec.encode(thrown?.stack),
  })
  const capabilities = {}
  for (const object of seeds) {
    const members = new Set()
    for (
      let prototype = object;
      prototype;
      prototype = Object.getPrototypeOf(prototype)
    )
      for (const name of Object.getOwnPropertyNames(prototype))
        if (name !== 'constructor') members.add(name)
    capabilities[object.constructor.name] = [...members].sort()
  }
  reply({
    event: 'ready',
    roots: { page: codec.encode(page), context: codec.encode(context) },
    capabilities,
  })
  // Never await a browser promise in this receive loop. A wait must leave room for the
  // click that satisfies it; settlement has a separate event and keeps native cancellation.
  return (async () => {
    for (;;) {
      const text = await __neo_test_host.receive()
      if (text === null) return
      const command = JSON.parse(text)
      if (command.op === 'finish') {
        reply({ id: command.id, kind: 'finished' })
        return
      }
      try {
        const object = resolve(command.object)
        const member = codec.string(command.member)
        if (command.op === 'has') {
          reply({
            id: command.id,
            kind: 'value',
            value: codec.encode(member in object),
          })
          continue
        }
        if (command.op === 'get') {
          if (!(member in object)) {
            reply({ id: command.id, kind: 'missing' })
            continue
          }
          const value = Reflect.get(object, member, object)
          reply(
            typeof value === 'function'
              ? { id: command.id, kind: 'callable' }
              : { id: command.id, kind: 'value', value: codec.encode(value) },
          )
          continue
        }
        if (command.op !== 'invoke' || typeof object[member] !== 'function')
          throw new ProtocolError('Invalid native invocation')
        const value = Reflect.apply(
          object[member],
          object,
          codec.decode(command.args),
        )
        if (value && typeof value.then === 'function') {
          reply({ id: command.id, kind: 'promise' })
          Promise.resolve(value)
            .then(
              (value) =>
                reply({
                  event: 'settled',
                  id: command.id,
                  ok: true,
                  value: codec.encode(value),
                }),
              (thrown) =>
                reply({
                  event: 'settled',
                  id: command.id,
                  ok: false,
                  ...error(thrown),
                }),
            )
            .catch((thrown) =>
              reply({
                event: 'transportError',
                id: command.id,
                message: String(thrown),
              }),
            )
        } else
          reply({ id: command.id, kind: 'value', value: codec.encode(value) })
      } catch (thrown) {
        if (codec.isError(thrown) || thrown instanceof ProtocolError)
          reply({
            id: command.id,
            kind: 'transportError',
            message: String(thrown),
          })
        else reply({ id: command.id, kind: 'throw', ...error(thrown) })
      }
    }
  })()
}

export const nativeScript = (epoch) =>
  `return (${nativeLoop.toString()})(${codecFactory.toString()}, ${JSON.stringify(epoch)})`
