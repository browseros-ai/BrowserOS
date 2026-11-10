/** Neo: The body of ONE native run. Reflection invokes actual SDK objects through the usual bridge. */
import { codecFactory } from './codec.mjs'

export function nativeLoop(codecFactory, epoch) {
  class ProtocolError extends Error {}
  const objects = new Map()
  const reverse = new WeakMap()
  // Methods can be shared across receivers. Send opaque identity plus public metadata,
  // then invoke the same actual function with the receiver supplied by the Node caller.
  const functions = new Map()
  const functionIds = new WeakMap()
  // Upstream's fixture defaults disable API timeouts. These are actual SDK setters,
  // applied before the measured body and inherited by pages created later in the run.
  context.setDefaultTimeout(0)
  context.setDefaultNavigationTimeout(0)
  page.setDefaultTimeout(0)
  page.setDefaultNavigationTimeout(0)
  const mainFrame = page.mainFrame()
  const seeds = [
    [page, 'Page'],
    [context, 'BrowserContext'],
    [mainFrame, 'Frame'],
    [page.locator('__neo_capability_probe__'), 'Locator'],
  ]
  const prototypes = new Map(
    seeds.map(([object, family]) => [Object.getPrototypeOf(object), family]),
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
  const codec = codecFactory({ identify, resolve, allowFunctions: false })
  const returned = (value, command) => {
    try {
      return codec.encode(value)
    } catch (thrown) {
      if (codec.isError(thrown))
        thrown.message = `bridge limit: untransportable return ${thrown.message.replace(/^Untransportable /, '')} from ${command.object.family}.${codec.string(command.member)}`
      throw thrown
    }
  }
  const reply = (frame) => __neo_test_host.reply(JSON.stringify(frame))
  const error = (thrown) => ({
    error: codec.encode({
      name: String(thrown?.name || 'Error'),
      message: String(thrown?.message ?? thrown),
    }),
    errorStack: codec.encode(thrown?.stack),
  })
  const capabilities = {}
  for (const [object, family] of seeds) {
    const members = new Set()
    for (
      let prototype = object;
      prototype;
      prototype = Object.getPrototypeOf(prototype)
    )
      for (const name of Object.getOwnPropertyNames(prototype))
        if (name !== 'constructor') members.add(name)
    capabilities[family] = [...members].sort()
  }
  reply({
    event: 'ready',
    roots: {
      page: codec.encode(page),
      context: codec.encode(context),
      mainFrame: codec.encode(mainFrame),
    },
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
          if (typeof value === 'function' && !functionIds.has(value)) {
            const id = functions.size + 1
            functions.set(id, value)
            functionIds.set(value, id)
          }
          reply(
            typeof value === 'function'
              ? {
                  id: command.id,
                  kind: 'callable',
                  function: {
                    id: functionIds.get(value),
                    name: codec.encode(value.name),
                    length: value.length,
                  },
                }
              : {
                  id: command.id,
                  kind: 'value',
                  value: returned(value, command),
                },
          )
          continue
        }
        const callable = functions.get(command.function)
        if (command.op !== 'invoke' || !callable)
          throw new ProtocolError('Invalid native invocation')
        const value = Reflect.apply(
          callable,
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
                  value: returned(value, command),
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
                message: codec.isError(thrown)
                  ? thrown.message
                  : String(thrown),
              }),
            )
        } else
          reply({
            id: command.id,
            kind: 'value',
            value: returned(value, command),
          })
      } catch (thrown) {
        if (codec.isError(thrown) || thrown instanceof ProtocolError)
          reply({
            id: command.id,
            kind: 'transportError',
            message: codec.isError(thrown) ? thrown.message : String(thrown),
          })
        else reply({ id: command.id, kind: 'throw', ...error(thrown) })
      }
    }
  })()
}

export const nativeScript = (epoch) =>
  `return (${nativeLoop.toString()})(${codecFactory.toString()}, ${JSON.stringify(epoch)})`
