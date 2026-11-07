/**
 * Fixture for neo-runtime's provenance tests, shaped like Playwright's isomorphic/yaml.ts.
 */

export function yamlStringNeedsQuotes(str: string): boolean {
  if (str.length === 0)
    return true;
  if (['y', 'n', 'yes', 'no', 'true', 'false', 'on', 'off', 'null', '~'].includes(str.toLowerCase()))
    return true;
  return false;
}
