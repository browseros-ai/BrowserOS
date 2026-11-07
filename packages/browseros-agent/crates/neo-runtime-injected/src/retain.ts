/**
 * Neo: the bundle entry `dist/retain.js`, `(object) => object`, for a page object an event
 * names (a console message's argument, in CDP's own object group). neo-runtime's
 * `main_world.rs` runs it with that object as its one argument and the run's handle group as
 * `objectGroup`, so CDP answers the same object under an id the run owns and releases with its
 * other handles. The object travels as an argument, not as `this`: the sloppy wrapper
 * (`build.ts`) would box a symbol `this`.
 */

export default function retain(object: unknown): unknown {
  return object
}
