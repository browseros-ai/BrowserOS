// Evaluated before bootstrap and preludes as a factory that
// takes four pure natives over Rust's `url` crate and defines URL and URLSearchParams on the
// global object. Every parse, setter and form encoding happens in Rust; this file is WebIDL glue:
// argument conversion, the URL <-> searchParams link, list operations and iteration.
// Linked mutations serialize the whole list, as the standard requires: for many appends, build a
// standalone URLSearchParams and assign url.search once to avoid quadratic work. The run heap
// and CPU budgets still apply; these natives perform no browser, host or network operations.
;(natives) => {
  const { parse, set, formParse, formSerialize } = natives
  // Capture intrinsics before any prelude or script can replace them. Preparing a linked
  // update must have no script callbacks after the arguments have been converted.
  const arrayPrototype = Array.prototype
  const { filter, find, findIndex, join, slice, some, sort } = arrayPrototype
  const { startsWith, slice: sliceString, toWellFormed } = String.prototype
  const parseJson = JSON.parse
  const apply = Reflect.apply
  const setPrototypeOf = Object.setPrototypeOf

  // WebIDL USVString: ToString (a Symbol throws a TypeError, as String() would not), then each lone
  // surrogate becomes U+FFFD. It is also the only form Rust's String accepts: QuickJS-ng hands a
  // lone surrogate to Rust as invalid UTF-8, which rquickjs refuses.
  const usv = (value) => apply(toWellFormed, `${value}`, [])

  // The natives' component order.
  const HREF = 0
  const ORIGIN = 1
  const PROTOCOL = 2
  const USERNAME = 3
  const PASSWORD = 4
  const HOST = 5
  const HOSTNAME = 6
  const PORT = 7
  const PATHNAME = 8
  const SEARCH = 9
  const HASH = 10

  // Lets URL.parse build an instance from parts it already holds; nothing outside this file has it.
  const FROM_PARTS = Symbol('fromParts')

  function required(count, given, what) {
    if (given < count)
      throw new TypeError(
        `Failed to ${what}: ${count} argument${count > 1 ? 's' : ''} required, but only ${given} present.`,
      )
  }

  function parseParts(url, base) {
    const json = parse(usv(url), base === undefined ? undefined : usv(base))
    return json === undefined ? undefined : parseJson(json)
  }

  function parseForm(search) {
    const text = apply(startsWith, search, ['?'])
      ? apply(sliceString, search, [1])
      : search
    return setPrototypeOf(parseJson(formParse(text)), null)
  }

  function sequencePairs(init, iterator) {
    // The union conversion already read @@iterator. Reuse that method and let the
    // nested for-of loops close their iterators if a conversion throws.
    const iterable = {
      [Symbol.iterator]: () => apply(iterator, init, []),
    }
    const pairs = setPrototypeOf([], null)
    for (const pair of iterable) {
      if (
        (typeof pair !== 'object' || pair === null) &&
        typeof pair !== 'function'
      )
        throw new TypeError(
          "Failed to construct 'URLSearchParams': The provided value cannot be converted to a sequence.",
        )
      const items = setPrototypeOf([], null)
      for (const item of pair) items[items.length] = usv(item)
      pairs[pairs.length] = items
    }

    // WebIDL converts the entire nested sequence before URLSearchParams checks pair
    // lengths. Stage on the capped JS heap and reuse the validated array as the list.
    for (let index = 0; index < pairs.length; index++) {
      if (pairs[index].length !== 2)
        throw new TypeError(
          "Failed to construct 'URLSearchParams': Sequence initializer must only contain pair elements",
        )
    }
    return pairs
  }

  // Wired by the classes' static blocks, which alone can reach the other's private fields.
  let applySearch // (url, serialized) — URLSearchParams' update steps
  let linkParams // (url, search) => URLSearchParams tied to url
  let refreshParams // (params, search) — install the list for a replacement query

  class URL {
    #parts
    #params = null

    constructor(url, base = undefined) {
      if (url === FROM_PARTS) {
        this.#parts = base
        return
      }
      required(1, arguments.length, "construct 'URL'")
      const parts = parseParts(url, base)
      if (parts === undefined)
        throw new TypeError("Failed to construct 'URL': Invalid URL")
      this.#parts = parts
    }

    static canParse(url, base = undefined) {
      required(1, arguments.length, "execute 'canParse' on 'URL'")
      return parseParts(url, base) !== undefined
    }

    static parse(url, base = undefined) {
      required(1, arguments.length, "execute 'parse' on 'URL'")
      const parts = parseParts(url, base)
      return parts === undefined ? null : new URL(FROM_PARTS, parts)
    }

    #nextParts(setter, text) {
      const json = set(this.#parts[HREF], setter, text)
      if (json !== undefined) return parseJson(json)
      if (setter === 'href')
        throw new TypeError(
          "Failed to set the 'href' property on 'URL': Invalid URL",
        )
      return this.#parts
    }

    #apply(setter, value) {
      // Conversion can reenter this URL. Read its current state only after conversion.
      const parts = this.#nextParts(setter, usv(value))
      // Prepare the linked list before publishing the new href. Native parsing and JS
      // allocation can fail, so only private field writes may follow a successful parse.
      if ((setter === 'href' || setter === 'search') && this.#params !== null)
        refreshParams(this.#params, parts[SEARCH])
      this.#parts = parts
    }

    get href() {
      return this.#parts[HREF]
    }
    set href(value) {
      // A private method call checks the receiver before any user value conversion.
      this.#apply('href', value)
    }
    get origin() {
      return this.#parts[ORIGIN]
    }
    get protocol() {
      return this.#parts[PROTOCOL]
    }
    set protocol(value) {
      this.#apply('protocol', value)
    }
    get username() {
      return this.#parts[USERNAME]
    }
    set username(value) {
      this.#apply('username', value)
    }
    get password() {
      return this.#parts[PASSWORD]
    }
    set password(value) {
      this.#apply('password', value)
    }
    get host() {
      return this.#parts[HOST]
    }
    set host(value) {
      this.#apply('host', value)
    }
    get hostname() {
      return this.#parts[HOSTNAME]
    }
    set hostname(value) {
      this.#apply('hostname', value)
    }
    get port() {
      return this.#parts[PORT]
    }
    set port(value) {
      this.#apply('port', value)
    }
    get pathname() {
      return this.#parts[PATHNAME]
    }
    set pathname(value) {
      this.#apply('pathname', value)
    }
    get search() {
      return this.#parts[SEARCH]
    }
    set search(value) {
      this.#apply('search', value)
    }
    get searchParams() {
      if (this.#params === null)
        this.#params = linkParams(this, this.#parts[SEARCH])
      return this.#params
    }
    get hash() {
      return this.#parts[HASH]
    }
    set hash(value) {
      this.#apply('hash', value)
    }
    toString() {
      return this.#parts[HREF]
    }
    toJSON() {
      return this.#parts[HREF]
    }

    static {
      applySearch = (url, serialized) => {
        url.#parts = url.#nextParts('search', serialized)
      }
    }
  }

  class URLSearchParams {
    // Internal arrays have no inherited hooks, including Array species used during copies.
    #list = setPrototypeOf([], null)
    #url = null

    constructor(init = '') {
      // WebIDL's optional union treats null as the empty dictionary.
      if (init === null) return
      if (
        (typeof init === 'object' && init !== null) ||
        typeof init === 'function'
      ) {
        const iterator = init[Symbol.iterator]
        if (iterator !== undefined && iterator !== null) {
          if (typeof iterator !== 'function')
            throw new TypeError(
              "Failed to construct 'URLSearchParams': The provided value is not iterable.",
            )
          this.#list = sequencePairs(init, iterator)
        } else {
          // Distinct property names can become the same USVString (lone surrogates).
          // WebIDL records keep the last value for a converted key, in its first position.
          const record = new Map()
          for (const key of Reflect.ownKeys(init)) {
            const descriptor = Reflect.getOwnPropertyDescriptor(init, key)
            if (descriptor !== undefined && descriptor.enumerable)
              record.set(usv(key), usv(init[key]))
          }
          for (const pair of record) this.#list[this.#list.length] = pair
        }
      } else {
        this.#list = parseForm(usv(init))
      }
    }

    #require(count, given, method) {
      // Calling this private method checks the receiver before argument conversion.
      required(count, given, `execute '${method}' on 'URLSearchParams'`)
    }

    #commit(list) {
      // A linked mutation stages its list until both serialization and URL normalization
      // succeed. Neither side can run script code between the two private field writes.
      if (this.#url !== null) applySearch(this.#url, this.#serialize(list))
      this.#list = list
    }

    #listForUpdate() {
      // Standalone appends keep their constant-time path.
      return this.#url === null
        ? this.#list
        : setPrototypeOf(apply(slice, this.#list, []), null)
    }

    #appendPair(pair) {
      const list = this.#listForUpdate()
      list[list.length] = pair
      this.#commit(list)
    }

    #serialize(list) {
      // Encoding primitive strings avoids inherited Array.prototype.toJSON hooks.
      // The capped JS heap holds the encoded list and joined string. Each native call
      // processes one bounded pair without an external pair vector or JSON decoder.
      const encoded = setPrototypeOf([], null)
      for (let index = 0; index < list.length; index++) {
        const pair = list[index]
        encoded[index] = formSerialize(pair[0], pair[1])
      }
      return apply(join, encoded, ['&'])
    }

    get size() {
      return this.#list.length
    }
    append(name, value) {
      this.#require(2, arguments.length, 'append')
      this.#appendPair([usv(name), usv(value)])
    }
    delete(name, value = undefined) {
      this.#require(1, arguments.length, 'delete')
      const key = usv(name)
      const only = value === undefined ? undefined : usv(value)
      const kept = apply(filter, this.#list, [
        (pair) =>
          !(pair[0] === key && (only === undefined || pair[1] === only)),
      ])
      this.#commit(setPrototypeOf(kept, null))
    }
    get(name) {
      this.#require(1, arguments.length, 'get')
      const key = usv(name)
      const pair = apply(find, this.#list, [(pair) => pair[0] === key])
      return pair === undefined ? null : pair[1]
    }
    getAll(name) {
      this.#require(1, arguments.length, 'getAll')
      const key = usv(name)
      const values = setPrototypeOf([], null)
      for (let index = 0; index < this.#list.length; index++) {
        const pair = this.#list[index]
        if (pair[0] === key) values[values.length] = pair[1]
      }
      return setPrototypeOf(values, arrayPrototype)
    }
    has(name, value = undefined) {
      this.#require(1, arguments.length, 'has')
      const key = usv(name)
      const only = value === undefined ? undefined : usv(value)
      return apply(some, this.#list, [
        (pair) => pair[0] === key && (only === undefined || pair[1] === only),
      ])
    }
    set(name, value) {
      this.#require(2, arguments.length, 'set')
      const key = usv(name)
      const text = usv(value)
      const index = apply(findIndex, this.#list, [(pair) => pair[0] === key])
      if (index === -1) {
        this.#appendPair([key, text])
        return
      }
      const kept = apply(filter, this.#list, [
        (pair, at) => at <= index || pair[0] !== key,
      ])
      kept[index] = [key, text]
      this.#commit(setPrototypeOf(kept, null))
    }
    sort() {
      // Array.prototype.sort is stable, and < compares code units, as the standard sorts.
      const list = this.#listForUpdate()
      apply(sort, list, [(a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)])
      this.#commit(list)
    }
    forEach(callback, thisArg = undefined) {
      this.#require(1, arguments.length, 'forEach')
      if (typeof callback !== 'function')
        throw new TypeError(
          "Failed to execute 'forEach' on 'URLSearchParams': The callback provided as parameter 1 is not a function.",
        )
      for (let index = 0; index < this.#list.length; index++) {
        const pair = this.#list[index]
        apply(callback, thisArg, [pair[1], pair[0], this])
      }
    }
    #iterate(pick) {
      return new ParamsIterator(() => this.#list, pick)
    }
    entries() {
      return this.#iterate((pair) => [pair[0], pair[1]])
    }
    keys() {
      return this.#iterate((pair) => pair[0])
    }
    values() {
      return this.#iterate((pair) => pair[1])
    }
    toString() {
      return this.#serialize(this.#list)
    }

    static {
      linkParams = (url, search) => {
        const params = new URLSearchParams(search)
        params.#url = url
        return params
      }
      refreshParams = (params, search) => {
        params.#list = parseForm(search)
      }
    }
  }

  // WebIDL iterators read the current list by index, including after a linked mutation
  // replaces the backing array. Exhaustion releases the reference to the owning params.
  class ParamsIterator {
    #readList
    #pick
    #index = 0
    constructor(readList, pick) {
      this.#readList = readList
      this.#pick = pick
    }
    next() {
      // Once exhausted, a WebIDL iterator stays done even if its old list grows later.
      if (this.#readList === null) return { value: undefined, done: true }
      const list = this.#readList()
      if (this.#index >= list.length) {
        this.#readList = null
        return { value: undefined, done: true }
      }
      return { value: this.#pick(list[this.#index++]), done: false }
    }
  }
  Reflect.setPrototypeOf(
    ParamsIterator.prototype,
    Reflect.getPrototypeOf(Reflect.getPrototypeOf([][Symbol.iterator]())),
  )
  Reflect.deleteProperty(ParamsIterator.prototype, 'constructor')
  Reflect.defineProperty(ParamsIterator.prototype, Symbol.toStringTag, {
    value: 'URLSearchParams Iterator',
    configurable: true,
  })
  URLSearchParams.prototype[Symbol.iterator] = URLSearchParams.prototype.entries

  // QuickJS-ng's own atob and btoa implement forgiving-base64 and InvalidCharacterError exactly,
  // but convert a missing argument to "undefined" where WebIDL throws a TypeError. One arity check
  // in front of each keeps the native for everything else.
  for (const name of ['atob', 'btoa']) {
    const native = globalThis[name]
    const checked = {
      [name](data) {
        required(1, arguments.length, `execute '${name}' on 'Window'`)
        return native(data)
      },
    }[name]
    Reflect.defineProperty(globalThis, name, {
      value: checked,
      writable: true,
      configurable: true,
    })
  }

  // WebIDL shape: attributes and operations are enumerable, the class string is the interface's
  // name, and the interface objects are writable, configurable, non-enumerable globals.
  for (const target of [
    URL,
    URL.prototype,
    URLSearchParams.prototype,
    ParamsIterator.prototype,
  ]) {
    for (const key of Reflect.ownKeys(target)) {
      if (
        typeof key === 'symbol' ||
        ['constructor', 'prototype', 'length', 'name'].includes(key)
      )
        continue
      const descriptor = Reflect.getOwnPropertyDescriptor(target, key)
      descriptor.enumerable = true
      Reflect.defineProperty(target, key, descriptor)
    }
  }
  for (const [name, value] of [
    ['URL', URL],
    ['URLSearchParams', URLSearchParams],
  ]) {
    Reflect.defineProperty(value.prototype, Symbol.toStringTag, {
      value: name,
      configurable: true,
    })
    Reflect.defineProperty(globalThis, name, {
      value,
      writable: true,
      configurable: true,
    })
  }
}
