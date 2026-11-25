import { chromium } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/node_modules/playwright-core/index.mjs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9334');
const context = browser.contexts()[0] ?? await browser.newContext();
const page = await context.newPage();
await page.goto('http://localhost:4174/mini-spiele', { waitUntil: 'domcontentloaded' });
await page.evaluate(() => { for (const k of Object.keys(localStorage)) if (k.startsWith('deadlockdle:')) localStorage.removeItem(k); });
await page.goto('http://localhost:4174/mini-spiele?spiel=guess-hero', { waitUntil: 'domcontentloaded' });
await page.waitForSelector('input[role=combobox]', { timeout: 60000 });
console.log(await page.evaluate(() => {
  const frame = document.querySelector('[role=img][data-state]') as HTMLElement;
  const probe = document.createElement('div');
  probe.style.filter = 'url(#x)';
  const blur = document.createElement('div');
  blur.style.filter = 'blur(2px)';
  return JSON.stringify({
    frameHtml: frame?.outerHTML.slice(0, 700),
    probe: probe.style.filter,
    blur: blur.style.filter,
    filterElements: document.querySelectorAll('filter').length,
    base: document.querySelector('base')?.outerHTML ?? null,
  });
}));
await page.close();
await browser.close();
