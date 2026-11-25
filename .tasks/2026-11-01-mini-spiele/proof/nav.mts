import { chromium } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/node_modules/playwright-core/index.mjs';

const [port, dev, csp, shotDir] = process.argv.slice(2);
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const context = browser.contexts()[0] ?? await browser.newContext();
const page = await context.newPage();
const problems: string[] = [];
page.on('pageerror', (e) => problems.push(`pageerror ${e.message}`));
page.on('console', (m) => { if (m.type() === 'error') problems.push(`console ${m.text()}`); });
const out: string[] = [];
const check = (name: string, ok: boolean, detail = '') => out.push(`${ok ? 'ok ' : 'FAIL'} ${name}${detail ? ` [${detail}]` : ''}`);

async function title() { await page.waitForSelector('h1', { state: 'attached', timeout: 30000 }); return page.locator('h1').first().textContent(); }

try {
  await page.goto(`${dev}/mini-spiele?lang=de`, { waitUntil: 'domcontentloaded' });
  await page.evaluate(() => { for (const k of Object.keys(localStorage)) if (k.startsWith('deadlockdle:')) localStorage.removeItem(k); localStorage.setItem('dashboard.language.v2', 'de'); });
  await page.goto(`${dev}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  check('hub title', (await title()) === 'Mini-Spiele');
  await page.locator('a[href*="spiel=guess-item"]').first().dispatchEvent('click');
  await page.waitForFunction(() => document.querySelector('h1')?.textContent === 'Errate das Item', undefined, { timeout: 30000 });
  check('tile opens game without reload', page.url().includes('spiel=guess-item'), page.url());
  await page.locator('a', { hasText: 'Zurück zur Übersicht' }).first().dispatchEvent('click');
  await page.waitForFunction(() => document.querySelector('h1')?.textContent === 'Mini-Spiele', undefined, { timeout: 30000 });
  check('back link returns to hub', !page.url().includes('spiel='), page.url());
  await page.locator('a[aria-label="Vorheriger Tag"]').first().dispatchEvent('click');
  await page.waitForFunction(() => location.search.includes('date='), undefined, { timeout: 10000 });
  check('previous day link keeps route', page.url().includes('/mini-spiele?date='), page.url());
  await page.goto(`${dev}/mini-spiele?spiel=guess-hero&date=2026-03-22`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('input[role=combobox]', { state: 'attached', timeout: 60000 });
  check('archive badge', (await page.locator('text=Archiv').count()) >= 1);
  await page.goto(`${dev}/mini-spiele?spiel=unbekannt`, { waitUntil: 'domcontentloaded' });
  check('unknown game shows hub', (await title()) === 'Mini-Spiele');
  await page.goto(`${dev}/mini-spiele?date=2999-01-01`, { waitUntil: 'domcontentloaded' });
  await title();
  check('future date falls back to today', (await page.locator('input[type=date]').inputValue()) === new Date().toISOString().slice(0, 10));

  await page.goto(`${csp}/mini-spiele?spiel=guess-ability`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('input[role=combobox]', { state: 'attached', timeout: 60000 });
  await page.waitForTimeout(2500);
  const img = await page.evaluate(() => { const i = document.querySelector('main img, img[alt*="Fähigkeit"], img[alt*="ability"]') as HTMLImageElement | null; const all = [...document.images].filter((x) => x.src.includes('deadlock-api')); return { n: all.length, loaded: all.filter((x) => x.complete && x.naturalWidth > 0).length, src: all[0]?.src.slice(0, 60) ?? i?.src ?? '' }; });
  check('CSP: API fetch ok and asset image loaded', img.n === 1 && img.loaded === 1, JSON.stringify(img));
  const blocked = await page.evaluate(async () => { try { await fetch('https://assets.deadlock-api.com/'); return 'allowed'; } catch { return 'blocked'; } });
  check('CSP: other host stays blocked (enforcement probe)', blocked === 'blocked', blocked);
  await page.screenshot({ path: `${shotDir}/csp-ability.png` });
} catch (error) {
  check('run aborted', false, String(error).slice(0, 400));
}
console.log(out.join('\n'));
console.log(`problems ${problems.length}`);
console.log([...new Set(problems)].slice(0, 10).join('\n').slice(0, 1200));
await page.close().catch(() => undefined);
await browser.close().catch(() => undefined);
