/** The orchestrator owns interruption, so aborting a child still passes through finalization. */
export function interruption() {
  const controller = new AbortController()
  const onInterrupt = () => controller.abort(new Error('interrupted by SIGINT'))
  const onTerminate = () =>
    controller.abort(new Error('interrupted by SIGTERM'))
  process.on('SIGINT', onInterrupt)
  process.on('SIGTERM', onTerminate)
  return {
    signal: controller.signal,
    dispose() {
      process.removeListener('SIGINT', onInterrupt)
      process.removeListener('SIGTERM', onTerminate)
    },
  }
}
