/*
 * Neo's main-world function with Playwright v1.64.1 (cfe490d941f992655948eee8fd4fd94e1c4234bf), packages/injected UtilityScript
 * and packages/isomorphic utilityScriptSerializers.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built unmodified from crates/browseros-playwright/third_party/playwright, with
 * packages/browseros-playwright-js/src/main-world, by packages/browseros-playwright-js/scripts/build.ts.
 * Do not edit. Notices: crates/browseros-playwright/THIRD_PARTY_NOTICES.md.
 */
function () {
'use strict';
// crates/browseros-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers.ts
var kFunctionBindingPrefix = "__pw_fn_";
var kBindingsControllerProperty = "__playwright__binding__controller__";
function isRegExp(obj) {
  try {
    return obj instanceof RegExp || Object.prototype.toString.call(obj) === "[object RegExp]";
  } catch (error) {
    return false;
  }
}
function isDate(obj) {
  try {
    return obj instanceof Date || Object.prototype.toString.call(obj) === "[object Date]";
  } catch (error) {
    return false;
  }
}
function isURL(obj) {
  try {
    return obj instanceof URL || Object.prototype.toString.call(obj) === "[object URL]";
  } catch (error) {
    return false;
  }
}
function isError(obj) {
  try {
    return obj instanceof Error || obj && Object.getPrototypeOf(obj)?.name === "Error";
  } catch (error) {
    return false;
  }
}
function isTypedArray(obj, constructor) {
  try {
    return obj instanceof constructor || Object.prototype.toString.call(obj) === `[object ${constructor.name}]`;
  } catch (error) {
    return false;
  }
}
function isArrayBuffer(obj) {
  try {
    return obj instanceof ArrayBuffer || Object.prototype.toString.call(obj) === "[object ArrayBuffer]";
  } catch (error) {
    return false;
  }
}
function isMap(obj) {
  try {
    return obj instanceof Map || Object.prototype.toString.call(obj) === "[object Map]";
  } catch (error) {
    return false;
  }
}
function isSet(obj) {
  try {
    return obj instanceof Set || Object.prototype.toString.call(obj) === "[object Set]";
  } catch (error) {
    return false;
  }
}
var typedArrayConstructors = {
  i8: Int8Array,
  ui8: Uint8Array,
  ui8c: Uint8ClampedArray,
  i16: Int16Array,
  ui16: Uint16Array,
  i32: Int32Array,
  ui32: Uint32Array,
  f32: Float32Array,
  f64: Float64Array,
  bi64: BigInt64Array,
  bui64: BigUint64Array
};
function typedArrayToBase64(array) {
  if ("toBase64" in array)
    return array.toBase64();
  const binary = Array.from(new Uint8Array(array.buffer, array.byteOffset, array.byteLength)).map((b) => String.fromCharCode(b)).join("");
  return btoa(binary);
}
function base64ToTypedArray(base64, TypedArrayConstructor) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0;i < binary.length; i++)
    bytes[i] = binary.charCodeAt(i);
  return new TypedArrayConstructor(bytes.buffer);
}
function parseEvaluationResultValue(value, handles = [], refs = new Map) {
  if (Object.is(value, undefined))
    return;
  if (typeof value === "object" && value) {
    if ("ref" in value)
      return refs.get(value.ref);
    if ("v" in value) {
      if (value.v === "undefined")
        return;
      if (value.v === "null")
        return null;
      if (value.v === "NaN")
        return NaN;
      if (value.v === "Infinity")
        return Infinity;
      if (value.v === "-Infinity")
        return -Infinity;
      if (value.v === "-0")
        return -0;
      return;
    }
    if ("d" in value) {
      return new Date(value.d);
    }
    if ("u" in value)
      return new URL(value.u);
    if ("bi" in value)
      return BigInt(value.bi);
    if ("e" in value) {
      const error = new Error(value.e.m);
      error.name = value.e.n;
      error.stack = value.e.s;
      return error;
    }
    if ("r" in value)
      return new RegExp(value.r.p, value.r.f);
    if ("a" in value) {
      const result = [];
      refs.set(value.id, result);
      for (const a of value.a)
        result.push(parseEvaluationResultValue(a, handles, refs));
      return result;
    }
    if ("o" in value) {
      const result = {};
      refs.set(value.id, result);
      for (const { k, v } of value.o) {
        if (k === "__proto__")
          continue;
        result[k] = parseEvaluationResultValue(v, handles, refs);
      }
      return result;
    }
    if ("m" in value) {
      const result = new Map;
      refs.set(value.id, result);
      for (const { k, v } of value.m)
        result.set(parseEvaluationResultValue(k, handles, refs), parseEvaluationResultValue(v, handles, refs));
      return result;
    }
    if ("s" in value) {
      const result = new Set;
      refs.set(value.id, result);
      for (const item of value.s)
        result.add(parseEvaluationResultValue(item, handles, refs));
      return result;
    }
    if ("h" in value)
      return handles[value.h];
    if ("fn" in value) {
      const name = value.fn;
      return (...args) => globalThis[kBindingsControllerProperty].callBinding(name, ...args);
    }
    if ("ta" in value)
      return base64ToTypedArray(value.ta.b, typedArrayConstructors[value.ta.k]);
    if ("ab" in value)
      return base64ToTypedArray(value.ab.b, Uint8Array).buffer;
  }
  return value;
}
function serializeAsCallArgument(value, handleSerializer, options = {}) {
  return serialize(value, handleSerializer, { visited: new Map, lastId: 0, extendedSerialization: options.extendedSerialization });
}
function serialize(value, handleSerializer, visitorInfo) {
  if (value && typeof value === "object") {
    if (typeof globalThis.Window === "function" && value instanceof globalThis.Window)
      return "ref: <Window>";
    if (typeof globalThis.Document === "function" && value instanceof globalThis.Document)
      return "ref: <Document>";
    if (typeof globalThis.Node === "function" && value instanceof globalThis.Node)
      return "ref: <Node>";
  }
  return innerSerialize(value, handleSerializer, visitorInfo);
}
function innerSerialize(value, handleSerializer, visitorInfo) {
  const result = handleSerializer(value);
  if ("fallThrough" in result)
    value = result.fallThrough;
  else
    return result;
  if (typeof value === "symbol")
    return { v: "undefined" };
  if (Object.is(value, undefined))
    return { v: "undefined" };
  if (Object.is(value, null))
    return { v: "null" };
  if (Object.is(value, NaN))
    return { v: "NaN" };
  if (Object.is(value, Infinity))
    return { v: "Infinity" };
  if (Object.is(value, -Infinity))
    return { v: "-Infinity" };
  if (Object.is(value, -0))
    return { v: "-0" };
  if (typeof value === "boolean")
    return value;
  if (typeof value === "number")
    return value;
  if (typeof value === "string")
    return value;
  if (typeof value === "bigint")
    return { bi: value.toString() };
  if (isError(value)) {
    let stack;
    if (value.stack?.startsWith(value.name + ": " + value.message)) {
      stack = value.stack;
    } else {
      stack = `${value.name}: ${value.message}
${value.stack}`;
    }
    return { e: { n: value.name, m: value.message, s: stack } };
  }
  if (isDate(value))
    return { d: value.toJSON() };
  if (isURL(value))
    return { u: value.toJSON() };
  if (isRegExp(value))
    return { r: { p: value.source, f: value.flags } };
  for (const [k, ctor] of Object.entries(typedArrayConstructors)) {
    if (isTypedArray(value, ctor))
      return { ta: { b: typedArrayToBase64(value), k } };
  }
  if (isArrayBuffer(value))
    return { ab: { b: typedArrayToBase64(new Uint8Array(value)) } };
  const id = visitorInfo.visited.get(value);
  if (id)
    return { ref: id };
  if (visitorInfo.extendedSerialization) {
    if (isMap(value)) {
      const m = [];
      const id = ++visitorInfo.lastId;
      visitorInfo.visited.set(value, id);
      const iterator = value.entries();
      const next = new Map().entries().next;
      while (true) {
        const entry = next.call(iterator);
        if (entry.done)
          break;
        m.push({ k: serialize(entry.value[0], handleSerializer, visitorInfo), v: serialize(entry.value[1], handleSerializer, visitorInfo) });
      }
      return { m, id };
    }
    if (isSet(value)) {
      const s = [];
      const id = ++visitorInfo.lastId;
      visitorInfo.visited.set(value, id);
      const iterator = value.values();
      const next = new Set().values().next;
      while (true) {
        const entry = next.call(iterator);
        if (entry.done)
          break;
        s.push(serialize(entry.value, handleSerializer, visitorInfo));
      }
      return { s, id };
    }
  }
  if (Array.isArray(value)) {
    const a = [];
    const id = ++visitorInfo.lastId;
    visitorInfo.visited.set(value, id);
    for (let i = 0;i < value.length; ++i)
      a.push(serialize(value[i], handleSerializer, visitorInfo));
    return { a, id };
  }
  if (typeof value === "object") {
    const o = [];
    const id = ++visitorInfo.lastId;
    visitorInfo.visited.set(value, id);
    for (const name of Object.keys(value)) {
      let item;
      try {
        item = value[name];
      } catch (e) {
        continue;
      }
      if (name === "toJSON" && typeof item === "function")
        o.push({ k: name, v: { o: [], id: 0 } });
      else
        o.push({ k: name, v: serialize(item, handleSerializer, visitorInfo) });
    }
    let jsonWrapper;
    try {
      if (o.length === 0 && value.toJSON && typeof value.toJSON === "function")
        jsonWrapper = { value: value.toJSON() };
    } catch (e) {}
    if (jsonWrapper)
      return innerSerialize(jsonWrapper.value, handleSerializer, visitorInfo);
    return { o, id };
  }
  if (typeof value === "function" && value.name.startsWith(kFunctionBindingPrefix))
    return { fn: value.name };
}

// crates/browseros-playwright/third_party/playwright/packages/injected/src/utilityScript.ts
class UtilityScript {
  global;
  builtins;
  isUnderTest;
  constructor(global, isUnderTest) {
    this.global = global;
    this.isUnderTest = isUnderTest;
    if (global.__pwClock) {
      this.builtins = global.__pwClock.builtins;
    } else {
      this.builtins = {
        setTimeout: global.setTimeout?.bind(global),
        clearTimeout: global.clearTimeout?.bind(global),
        setInterval: global.setInterval?.bind(global),
        clearInterval: global.clearInterval?.bind(global),
        requestAnimationFrame: global.requestAnimationFrame?.bind(global),
        cancelAnimationFrame: global.cancelAnimationFrame?.bind(global),
        requestIdleCallback: global.requestIdleCallback?.bind(global),
        cancelIdleCallback: global.cancelIdleCallback?.bind(global),
        performance: global.performance,
        Intl: global.Intl,
        Date: global.Date,
        AbortSignal: global.AbortSignal,
        Temporal: global.Temporal
      };
    }
    if (this.isUnderTest)
      global.builtins = this.builtins;
  }
  evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) {
    const args = argsAndHandles.slice(0, argCount);
    const handles = argsAndHandles.slice(argCount);
    const parameters = [];
    for (let i = 0;i < args.length; i++)
      parameters[i] = parseEvaluationResultValue(args[i], handles);
    let result = this.global.eval(expression);
    if (isFunction === true) {
      result = result(...parameters);
    } else if (isFunction === false) {
      result = result;
    } else {
      if (typeof result === "function")
        result = result(...parameters);
    }
    return returnByValue ? this._promiseAwareJsonValueNoThrow(result) : result;
  }
  jsonValue(returnByValue, value) {
    if (value === undefined)
      return;
    return serializeAsCallArgument(value, (value) => ({ fallThrough: value }));
  }
  _promiseAwareJsonValueNoThrow(value) {
    const safeJson = (value) => {
      try {
        return this.jsonValue(true, value);
      } catch (e) {
        return;
      }
    };
    if (value && typeof value === "object" && typeof value.then === "function") {
      return (async () => {
        const promiseValue = await value;
        return safeJson(promiseValue);
      })();
    }
    return safeJson(value);
  }
}

// packages/browseros-playwright-js/src/main-world/index.ts
var isThenable = (value) => !!value && typeof value === "object" && typeof value.then === "function";
var asText = (value) => value === undefined ? undefined : JSON.stringify(value);
function neoMainWorld(mode, isFunction, expression, argCount, ...argTextsThenHandles) {
  const args = argTextsThenHandles.slice(0, argCount).map((text) => JSON.parse(text));
  const handles = argTextsThenHandles.slice(argCount);
  const utility = new UtilityScript(globalThis, false);
  if (mode === "evaluate") {
    const out = utility.evaluate(isFunction, true, expression, argCount, ...args, ...handles);
    return isThenable(out) ? out.then((value) => asText(value)) : asText(out);
  }
  const [arg] = args.map((serialized) => parseEvaluationResultValue(serialized, handles));
  let evaledExpression;
  const predicate = () => {
    let result = evaledExpression ?? globalThis.eval(expression);
    if (isFunction === true || isFunction !== false && typeof result === "function") {
      evaledExpression = result;
      result = result(arg);
    }
    return result;
  };
  const poll = () => {
    const success = predicate();
    if (!success)
      return [false];
    return new Promise((fulfill) => fulfill(success)).then((value) => {
      poll.last = value;
      return [true, asText(utility.jsonValue(true, value))];
    });
  };
  poll.first = poll();
  return poll;
}
return neoMainWorld.apply(this, arguments);
}