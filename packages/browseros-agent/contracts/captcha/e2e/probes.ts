/** Probe tags let the local stub answer without evaluating arbitrary JavaScript. */
export const probes = {
  ready: '/* captcha-e2e:ready */',
  observation: '/* captcha-e2e:observation */',
  submit: '/* captcha-e2e:submit */',
  body: '/* captcha-e2e:body */',
}
export const readyProbe = (expression: string) =>
  `${probes.ready} return Boolean(${expression})`
export const observationProbe = (expression: string) => `${probes.observation}
  const fixture = JSON.parse(document.documentElement.dataset.captchaFixture || 'null');
  return { tokenLength: Number(${expression}), callbackFired: fixture?.callbackFired ?? null,
    fixtureTokenLength: fixture?.tokenLength ?? null };`
export const submitProbe = (selector: string) => `${probes.submit}
  const button = document.querySelector(${JSON.stringify(selector)});
  if (!button) throw new Error('submit control missing'); button.click(); return true;`
export const bodyProbe = `${probes.body} return document.body.innerText`
