# Application lifecycle and trace draining

The application or deployment runtime owns termination. Reuse its existing
lifecycle mechanism and make AgentPond the final telemetry step after the
application stops producing spans. Do not add a second signal owner when the
framework, process manager, or application already has one.

Classify the runtime before choosing a pattern.

## One-shot scripts, jobs, and tests

Start the SDK once and await shutdown in `finally` so success and handled-error
paths both drain queued spans:

```ts
sdk.start();

try {
	await runApplication();
} finally {
	await sdk.shutdown();
}
```

Use the corresponding provider or processor shutdown when the application
constructs OpenTelemetry manually. Do not call `process.exit()` after starting
shutdown; an immediate exit can abandon the pending promise and trace writes.

## Long-running Node.js servers

Prefer the framework's existing graceful-shutdown hook. For example, a
Fastify-owned provider can shut down after the server stops accepting work:

```ts
sdk.start();

app.addHook("onClose", async () => {
	await sdk.shutdown();
});
```

Follow the application's established shutdown order: stop accepting requests,
finish in-flight work, close other span-producing resources, and then shut down
OpenTelemetry. If the application owns `SIGINT` or `SIGTERM`, extend that one
central path instead of registering AgentPond signal handlers in a reusable
module. Keep its existing error logging and exit semantics.

Do not use an async `process.on("exit", ...)` handler; Node cannot await it.
`beforeExit` is also not a substitute for graceful shutdown because it is not
emitted for every termination path and asynchronous work can cause it to run
again.

## Reusable request and serverless runtimes

Keep the module-level provider alive between requests. End the request's spans,
then attach `forceFlush()` to the platform's native request-lifetime primitive:

```ts
after(() => processor.forceFlush());
```

```ts
context.waitUntil(processor.forceFlush());
```

```ts
EdgeRuntime.waitUntil(processor.forceFlush());
```

Retain the explicit `BatchSpanProcessor` used by these calls. Handle rejected
flushes through the application's normal logging conventions. Do not shut down
a reusable provider after every request, and do not block the response when the
runtime supplies a supported background-lifetime primitive.

## End-to-end validation

Validate the same boundary production uses after a representative AI request:

- Let a one-shot process reach and await its `finally` shutdown.
- Invoke a server's normal graceful-stop path rather than calling telemetry
  shutdown directly from the test.
- Run or mock the serverless request-lifetime primitive and await the promise or
  deferred callback it received.

Then sync AgentPond and read the new trace back from the selected store. A
build, typecheck, unit test, or direct call to `forceFlush()` without exercising
the application's real boundary does not validate lifecycle integration.
