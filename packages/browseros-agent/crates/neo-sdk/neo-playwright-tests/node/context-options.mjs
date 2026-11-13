/** Neo: Omit only the two unsupported implicit defaults; explicit test options reach the SDK. */

export function contextOptions(combined, requested, implicit) {
  const options = { ...combined, ...requested }
  for (const [name, value] of [
    ['offline', false],
    ['acceptDownloads', true],
  ]) {
    if (
      implicit?.has(name) &&
      !Object.hasOwn(requested || {}, name) &&
      options[name] === value
    )
      delete options[name]
  }
  return options
}
