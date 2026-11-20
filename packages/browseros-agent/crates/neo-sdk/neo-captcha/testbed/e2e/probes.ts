import type { SiteCheck } from '../fixtures/catalog'

/** In-page function bodies. Their tags let the local stub answer without evaluating them. */
export const probes = {
  ready: '/* captcha-e2e:ready */',
  observation: '/* captcha-e2e:observation */',
  submit: '/* captcha-e2e:submit */',
  body: '/* captcha-e2e:body */',
  trigger: '/* captcha-e2e:trigger */',
  reading: '/* captcha-e2e:reading */',
}
export const readyProbe = (expression: string) =>
  `${probes.ready} return Boolean(${expression})`
export const observationProbe = (expression: string) => `${probes.observation}
  const fixture = JSON.parse(document.documentElement.dataset.captchaFixture || 'null');
  return { tokenLength: Number(${expression}), callbackFired: fixture?.callbackFired ?? null,
    fixtureTokenLength: fixture?.tokenLength ?? null };`
export const clickProbe = (selector: string) => `
  const button = document.querySelector(${JSON.stringify(selector)});
  if (!button) throw new Error('control missing: ' + ${JSON.stringify(selector)}); button.click(); return true;`
/**
 * Submits the page's form. Skipped when the page has no submit control, or when it already shows
 * its acceptance: a widget callback (invisible reCAPTCHA's) may have submitted the form itself.
 */
export const submitProbe = (site: SiteCheck) => `${probes.submit}
  if (new RegExp(${JSON.stringify(site.acceptedPattern)}).test(document.body?.innerText ?? '')) return false;
  ${site.submitSelector ? clickProbe(site.submitSelector) : 'return false;'}`
/** A recorded page value: a string, or null while it is not there. */
export const readingProbe = (expression: string) =>
  `${probes.reading} const value = (${expression}); return typeof value === 'string' ? value.slice(0, 300) : null;`
export const bodyProbe = (site: SiteCheck) => `${probes.body}
  const text = document.body?.innerText ?? '';
  if (new RegExp(${JSON.stringify(site.acceptedPattern)}).test(text)) return 'accepted';
  const rejected = ${JSON.stringify(site.rejectedPattern ?? null)};
  return rejected && new RegExp(rejected).test(text) ? 'rejected' : null;`
