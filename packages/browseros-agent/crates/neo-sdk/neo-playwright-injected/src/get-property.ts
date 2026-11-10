/**
 * Playwright: `server/javascript.ts` `JSHandle._getProperty`
 *
 * The bundle entry `dist/get-property.js`: the page function of `JSHandle.getProperty`, which
 * copies one property (own, inherited or a getter's value) onto a null-prototype object that
 * neo-runtime then reads with `Runtime.getProperties`. neo-runtime's `main_world.rs` passes it as
 * the `expression` of `UtilityScript.evaluate` (`dist/evaluate.js`), with the handle and the name
 * as its arguments, so a primitive handle reads its property as Playwright's does.
 */

export default function getProperty(
  object: Record<string, unknown>,
  propertyName: string,
): Record<string, unknown> {
  const result: Record<string, unknown> = Object.create(null)
  result[propertyName] = object[propertyName]
  return result
}
