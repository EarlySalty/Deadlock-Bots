import { chromium } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/node_modules/playwright-core/index.mjs';
import { heroPoolOn, itemPoolOn } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/src/minigames/lib/deadlockdle/roster';
import { buildGuessableAbilities } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/src/minigames/lib/deadlockdle/hints';
import { getModeSeed, getTodayDate, seededPick, seededRandom } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/src/minigames/lib/deadlockdle/seed';
import { displayName } from '/home/nathanael/.worktrees/tb-mini-spiele-tab/bot/dashboard_v2/src/minigames/lib/api';

const [port, base, shotDir] = process.argv.slice(2);
const API = 'https://api.deadlock-api.com';
const LANG = { de: 'german', en: 'english' } as const;
const LABEL = { de: 'Deutsch', en: 'English' } as const;
const clean = <T extends { name: string }>(list: T[]) => list.map((e) => ({ ...e, name: displayName(e.name) }));
const get = async (path: string) => { const r = await fetch(`${API}${path}`); if (!r.ok) throw new Error(`${path} ${r.status}`); return r.json(); };

async function pools(lang: 'de' | 'en') {
  const date = getTodayDate();
  const language = LANG[lang];
  const heroes = clean(await get(`/v1/assets/heroes?language=${language}&only_active=false`));
  const items = clean(await get(`/v1/assets/items/by-type/upgrade?language=${language}`));
  const abilities = clean(await get(`/v1/assets/items/by-type/ability?language=${language}`));
  const heroPool = heroPoolOn(heroes, date);
  return {
    'guess-hero': heroPool,
    'guess-item': itemPoolOn(items, date),
    'guess-ability': buildGuessableAbilities(abilities, heroPool),
  } as Record<string, { id: number; name: string }[]>;
}

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const context = browser.contexts()[0] ?? await browser.newContext();
const page = await context.newPage();
const problems: string[] = [];
page.on('pageerror', (e) => problems.push(`pageerror ${e.message}`));
page.on('console', (m) => { if (m.type() === 'error') problems.push(`console ${m.text()}`); });
page.on('requestfailed', (r) => problems.push(`failed ${r.url()}`));

const out: string[] = [];
const check = (name: string, ok: boolean, detail = '') => { out.push(`${ok ? 'ok ' : 'FAIL'} ${name}${detail ? ` [${detail}]` : ''}`); };
const finish = () => {
  console.log(out.join('\n'));
  console.log(`problems ${problems.length}`);
  console.log([...new Set(problems)].slice(0, 15).join('\n').slice(0, 1500));
};

async function setSize(w: number, h: number) { await page.setViewportSize({ width: w, height: h }); }
async function open(mode: string) {
  await page.goto(`${base}/mini-spiele?spiel=${mode}`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('input[role=combobox]', { timeout: 60000 });
}
async function guessNames() { return page.locator('[data-testid=previous-guesses] li').allTextContents(); }
async function switchLang(lang: 'de' | 'en') {
  await page.locator(`[data-language-switch] button[title="${LABEL[lang]}"]`).first().dispatchEvent("click");
  await page.waitForTimeout(300);
}
async function enter(name: string) {
  const input = page.locator('input[role=combobox]');
  await input.fill(name);
  await page.waitForSelector('[role=option]', { state: 'attached', timeout: 5000 });
  const first = await page.locator('[role=option]').first().textContent();
  if (first === name) {
    await input.press('Enter');
  } else {
    await page.locator('[role=option]', { hasText: name }).first().dispatchEvent('click');
  }
  await page.waitForTimeout(150);
}
async function clearProgress(lang: 'de' | 'en') {
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.evaluate((l) => {
    for (const k of Object.keys(localStorage)) if (k.startsWith('deadlockdle:')) localStorage.removeItem(k);
    localStorage.setItem('dashboard.language.v2', l);
  }, lang);
}

try {
  const de = await pools('de');
  const en = await pools('en');
  const today = getTodayDate();
  for (const mode of Object.keys(de)) {
    const ans = seededPick(de[mode], seededRandom(getModeSeed(today, mode)));
    const ansEn = seededPick(en[mode], seededRandom(getModeSeed(today, mode)));
    check(`${mode} same answer id across languages`, ans.id === ansEn.id, `${ans.id} ${ans.name} / ${ansEn.name}`);
  }

  for (const start of ['de', 'en'] as const) {
    const other = start === 'de' ? 'en' : 'de';
    const startPool = start === 'de' ? de : en;
    const otherPool = start === 'de' ? en : de;
    await setSize(1440, 900);
    await clearProgress(start);
    for (const mode of Object.keys(de)) {
      const answer = seededPick(startPool[mode], seededRandom(getModeSeed(today, mode)));
      const wrong = startPool[mode].find((e) => e.id !== answer.id && e.name.toLowerCase() !== answer.name.toLowerCase())!;
      const wrongOther = otherPool[mode].find((e) => e.id === wrong.id)!;
      const answerOther = otherPool[mode].find((e) => e.id === answer.id)!;
      await page.evaluate((l) => localStorage.setItem('dashboard.language.v2', l), start);
      await open(mode);
      await enter(wrong.name);
      if (mode === 'guess-hero') {
        const art = await page.evaluate(() => { const f = document.querySelector('[role=img][data-state=hidden] span') as HTMLElement | null; const id = f?.style.filter.match(/#([^"')]+)/)?.[1]; return { filter: f?.style.filter ?? '', present: Boolean(id && document.getElementById(id)) }; });
        check(`${start} hero silhouette filter wired`, art.present, art.filter);
      }
      check(`${start} ${mode} wrong guess listed`, (await guessNames()).join() === wrong.name, wrong.name);
      check(`${start} ${mode} wrong marked`, (await page.locator('[data-result=wrong]').count()) === 1);
      await page.reload({ waitUntil: 'domcontentloaded' });
      await page.waitForSelector('[data-testid=previous-guesses]', { state: 'attached', timeout: 60000 });
      check(`${start} ${mode} guess survives reload`, (await guessNames()).join() === wrong.name);
      await switchLang(other);
      await page.waitForFunction((n) => document.querySelector('[data-testid=previous-guesses] li')?.textContent === n, wrongOther.name, { timeout: 60000 });
      check(`${start}->${other} ${mode} guess relocalized mid puzzle`, true, wrongOther.name);
      await page.reload({ waitUntil: 'domcontentloaded' });
      await page.waitForSelector('[data-testid=previous-guesses]', { state: 'attached', timeout: 60000 });
      check(`${start}->${other} ${mode} after switch and reload`, (await guessNames()).join() === wrongOther.name);
      const statusText = await page.locator('[data-testid=attempts-status]').textContent();
      check(`${other} ${mode} attempts status`, /\b[45]\b/.test(statusText ?? ''), statusText ?? '');
      if (mode === 'guess-hero' && start === 'de') {
        await page.waitForTimeout(800);
        await page.screenshot({ path: `${shotDir}/hero-mid-${other}-1440.png` });
        await setSize(390, 844);
        await page.waitForTimeout(400);
        await page.screenshot({ path: `${shotDir}/hero-mid-${other}-390.png` });
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
        check(`${other} hero mobile no horizontal overflow`, overflow <= 0, String(overflow));
        await setSize(1440, 900);
      }
      await enter(answerOther.name);
      await page.waitForSelector('[data-testid=game-result]', { state: 'attached', timeout: 5000 });
      check(`${other} ${mode} won`, (await page.locator('[data-result=correct]').count()) === 1, answerOther.name);
      check(`${other} ${mode} answer shown`, (await page.locator('[data-testid=game-answer]').textContent()) === answerOther.name);
      await page.waitForTimeout(800);
      await page.reload({ waitUntil: 'domcontentloaded' });
      await page.waitForSelector('[data-testid=game-result]', { state: 'attached', timeout: 60000 });
      check(`${other} ${mode} result survives reload`, (await page.locator('input[role=combobox]').isDisabled()) && (await guessNames()).length === 2);
      await switchLang(start);
      await page.waitForFunction((n) => document.querySelector('[data-testid=game-answer]')?.textContent === n, answer.name, { timeout: 60000 });
      check(`${start} ${mode} finished state relocalized`, true, answer.name);
      await switchLang(other);
      await page.waitForTimeout(1200);
      await page.screenshot({ path: `${shotDir}/${mode}-won-${other}-1440.png`, fullPage: true });
      await setSize(390, 844);
      await page.waitForTimeout(400);
      await page.screenshot({ path: `${shotDir}/${mode}-won-${other}-390.png`, fullPage: true });
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
      check(`${other} ${mode} result mobile no overflow`, overflow <= 0, String(overflow));
      await setSize(1440, 900);
    }
    await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('h1', { timeout: 30000 });
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `${shotDir}/hub-${other}-1440.png`, fullPage: true });
    check(`${other} hub credit`, (await page.locator('a[href="https://deadlock-api.com"]').count()) === 1);
    check(`${other} hub tiles`, (await page.locator('a[href*="spiel="]').count()) === 3);
    await setSize(390, 844);
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${shotDir}/hub-${other}-390.png`, fullPage: true });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(`${other} hub mobile no overflow`, overflow <= 0, String(overflow));
  }

  await setSize(1440, 900);
  await clearProgress('de');
  await open('guess-item');
  const pool = de['guess-item'];
  const a = seededPick(pool, seededRandom(getModeSeed(today, 'guess-item')));
  for (const e of pool.filter((x) => x.id !== a.id).slice(0, 5)) await enter(e.name);
  await page.waitForSelector('[data-testid=game-result]', { state: 'attached', timeout: 5000 });
  check('de guess-item loss after 5', (await page.locator('[data-testid=game-answer]').textContent()) === a.name);
  await page.waitForTimeout(1200);
  await page.screenshot({ path: `${shotDir}/item-lost-de-1440.png`, fullPage: true });
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector('[data-testid=game-result]', { state: 'attached', timeout: 60000 });
  check('de guess-item loss survives reload', (await guessNames()).length === 5);
} catch (error) {
  check('run aborted', false, String(error).slice(0, 400));
  await page.screenshot({ path: `${shotDir}/abort.png` }).catch(() => undefined);
}
finish();
await page.close().catch(() => undefined);
await browser.close().catch(() => undefined);
