const W = '/home/nathanael/.worktrees/tb-mini-spiele-p3/bot/dashboard_v2/';
const L = W + 'src/minigames/lib/';
const { chromium } = await import(W + 'node_modules/playwright-core/index.mjs');
const { displayName, fetchApiList } = await import(L + 'api.ts');
const { getTodayDate, getDayNumber } = await import(L + 'deadlockdle/seed.ts');
const { heroPool, itemPool, upgradePathPool, heroIconSrc, itemIconSrc } = await import(L + 'flashcards/pools.ts');
const { effectSections } = await import(L + 'flashcards/item-effects.ts');
const { gameTermsFor } = await import(L + 'flashcards/game-terms.ts');

const [moliPort, base, shotDir, sectionList = 'hub,heroes,decks,effects,edge,result,share,hints,terms,tiles'] = process.argv.slice(2);
const sections = new Set(sectionList.split(','));
const today = getTodayDate();
const dayBefore = (n: number) => new Date(Date.parse(`${today}T00:00:00Z`) - n * 86400000).toISOString().slice(0, 10);
const LABEL = { de: 'Deutsch', en: 'English' } as const;
type Lang = 'de' | 'en';

const clean = (list: any[]) => list.map((entry) => ({ ...entry, name: displayName(entry.name) }));
const assets = async (path: string, params: Record<string, string>) => clean(await fetchApiList(path, params));
const heroesEn = await assets('/v1/assets/heroes', { only_active: 'false', language: 'english' });
const heroesDe = await assets('/v1/assets/heroes', { only_active: 'false', language: 'german' });
const itemsEn = await assets('/v1/assets/items/by-type/upgrade', { language: 'english' });
const itemsDe = await assets('/v1/assets/items/by-type/upgrade', { language: 'german' });
const heroDeck = heroPool(heroesEn);
const itemDeck = itemPool(itemsEn);
const anyTerms = gameTermsFor('english');
const effectDeck = itemDeck.filter((item: any) => effectSections(item, anyTerms).length > 0);
const upgradeDeck = upgradePathPool(itemsEn);
const deName = new Map(heroesDe.map((hero: any) => [hero.id, hero.name]));
const itemDe = new Map(itemsDe.map((item: any) => [item.id, item]));
const heroBySrc = new Map(heroDeck.map((hero: any) => [heroIconSrc(hero), hero]));
const itemBySrc = new Map(itemDeck.map((item: any) => [itemIconSrc(item), item]));

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${moliPort}`);
const context = browser.contexts()[0] ?? (await browser.newContext());
const page = await context.newPage();
const WIDTH = Number(process.env.P2_WIDTH ?? 1440);
await page.setViewportSize({ width: WIDTH, height: WIDTH < 500 ? 844 : 900 });
if (WIDTH < 640) await page.addInitScript(() => { addEventListener('DOMContentLoaded', () => { const tag = document.createElement('style'); tag.textContent = 'dd > span.invisible{display:none!important} dl dd{display:block!important}'; document.head.appendChild(tag); }); });
await page.addInitScript('globalThis.__name = (fn) => fn');
const problems: string[] = [];
const requests: string[] = [];
let aborted = 0;
page.on('pageerror', (error: Error) => { problems[problems.length] = `pageerror ${error.message}`; });
page.on('console', (message: any) => { if (message.type() === 'error') problems[problems.length] = `console ${message.text()}`; });
page.on('request', (request: any) => { requests[requests.length] = request.url(); });
page.on('requestfailed', (request: any) => {
  if ((request.failure()?.errorText ?? '').includes('ERR_ABORTED')) aborted += 1;
  else problems[problems.length] = `failed ${request.url()} ${request.failure()?.errorText}`;
});

const note = (line: string) => console.log(`    ${line}`);
let failures = 0;
const check = (name: string, ok: boolean, detail: unknown = '') => { if (!ok) failures += 1; console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail !== '' ? ` [${String(detail).slice(0, 400)}]` : ''}`); };
const sleep = (ms: number) => page.waitForTimeout(ms);
const settle = () => sleep(350);
const OPTION = '[data-answer-option]';
const STAT = '[data-testid=flashcard-stat]';
const ANNOUNCE = '[data-testid=flashcard-announcement]';
const PROMPT = '[data-testid=flashcard-prompt]';
const MASTERED = '[data-testid=flashcard-mastered]';

async function shot(name: string) {
  await sleep(300);
  const overflow = await page.evaluate(() => {
    const limit = window.innerWidth;
    const offenders: string[] = [];
    for (const element of document.querySelectorAll('body *')) {
      let clipped = false;
      for (let parent = element.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        if (getComputedStyle(parent).overflowX !== 'visible') clipped = true;
      }
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.right > limit + 1 && getComputedStyle(element).visibility !== 'hidden' && !clipped && !element.closest('.sr-only') && offenders.length < 4) offenders[offenders.length] = `${element.tagName}.${String(element.getAttribute('class')).slice(0, 60)}:${Math.round(rect.right)}`;
    }
    return { width: limit, scroll: document.documentElement.scrollWidth, offenders };
  });
  check(`${name} no horizontal overflow at ${WIDTH}`, overflow.offenders.length === 0 && overflow.scroll <= overflow.width, JSON.stringify(overflow));
  if (process.env.P2_NOSHOT) return;
  const taken = page.screenshot({ path: `${shotDir}/p3-${name}-${WIDTH}.png`, fullPage: true, timeout: 25000 }).then(() => true).catch(() => false);
  const ok = await Promise.race([taken, new Promise((done) => setTimeout(() => done(false), 30000))]);
  if (!ok) note(`screenshot failed ${name}`);
}
async function reset(lang: Lang, terms: 'english' | 'german' = 'english', seed: Record<string, unknown> = {}) {
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="spiel="]', { state: 'attached', timeout: 60000 });
  await page.evaluate(([language, gameLanguage, entries]: [string, string, Record<string, unknown>]) => {
    for (const name of Object.keys(localStorage)) if (name.startsWith('deadlockdle:') || name.startsWith('flashcards:')) localStorage.removeItem(name);
    localStorage.setItem('dashboard.language.v2', language);
    localStorage.setItem('deadlockdle:game-language', JSON.stringify(gameLanguage));
    for (const [key, value] of Object.entries(entries)) localStorage.setItem(key, JSON.stringify(value));
  }, [lang, terms, seed]);
}
const openDeck = async (deck: string) => {
  await page.goto(`${base}/mini-spiele?spiel=flashcards&stapel=${deck}`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector(`${OPTION}, ${MASTERED}`, { state: 'attached', timeout: 90000 });
  await settle();
};
const reloadDeck = async () => {
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector(`${OPTION}, ${MASTERED}`, { state: 'attached', timeout: 90000 });
  await settle();
};
async function switchLang(lang: Lang) {
  await page.locator(`[data-language-switch] button[title="${LABEL[lang]}"]`).first().dispatchEvent('click');
  await sleep(400);
}
async function switchTerms(value: 'english' | 'german') {
  await page.locator(`[data-game-terms-switch] button[data-value=${value}]`).first().dispatchEvent('click');
  await sleep(600);
}
const pageText = () => page.evaluate(() => (document.querySelector('main') ?? document.body as HTMLElement).innerText);
const texts = (selector: string) => page.evaluate((s: string) => [...document.querySelectorAll(s)].map((node) => (node.textContent ?? '').trim()), selector);
const promptSrc = () => page.evaluate((s: string) => document.querySelector(`${s} img`)?.getAttribute('src') ?? '', PROMPT);
const stored = (key: string) => page.evaluate((name: string) => JSON.parse(localStorage.getItem(name) ?? 'null'), key);
const attributeLeak = (needles: string[]) => page.evaluate((list: string[]) => {
  const found: string[] = [];
  for (const element of document.querySelectorAll('main *')) {
    for (const { name, value } of element.attributes) {
      if (name === 'src' || name === 'class' || name === 'style') continue;
      if (value && list.some((needle) => value.includes(needle))) found[found.length] = `${name}=${value}`;
    }
  }
  return found;
}, needles);
const clickOption = (index: number) => page.locator(OPTION).nth(index).dispatchEvent('click');
const nextCard = async () => { const button = page.locator('[data-state=revealed] button'); if (await button.count()) await button.first().dispatchEvent('click'); await sleep(850); };
const correctIndexFor = async (names: string[]) => {
  const options = await texts(OPTION);
  return options.findIndex((text: string) => names.some((name) => text.replace(/^\d/, '').trim() === name || text.includes(name)));
};
const heroNames = (hero: any) => [hero.name, deName.get(hero.id)].filter(Boolean) as string[];
const allHeroNames = heroDeck.flatMap(heroNames);
const fresh = { correct: 0, seen: 0, streak: 0, bestStreak: 0 };
const nearEnd = (deck: string, ids: number[]) => ({
  [`flashcards:${deck}:no-repeats`]: true,
  [`flashcards:${deck}:progress`]: { stats: { ...fresh, correct: ids.length, seen: ids.length, streak: ids.length, bestStreak: ids.length }, seenIds: ids },
});

async function hub() {
  await reset('de');
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="spiel="]', { state: 'attached', timeout: 60000 });
  await settle();
  const tiles = await page.locator('a[href*="spiel="]').count();
  check('hub shows 8 tiles', tiles === 8, tiles);
  const body = await pageText();
  check('footer removed', !/deadlock-api\.com|MIT-Lizenz/.test(body));
  await shot('hub-de');
  await page.goto(`${base}/mini-spiele?spiel=flashcards`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="stapel="]', { state: 'attached', timeout: 60000 });
  await settle();
  check('flashcards hub shows 4 decks', await page.locator('a[href*="stapel="]').count() === 4);
  check('flashcards hub has the terms switch', await page.locator('[data-game-terms-switch]').count() > 0);
  await shot('flashcards-hub-de');
  await switchLang('en');
  await shot('flashcards-hub-en');
}

async function heroes(lang: Lang) {
  await reset(lang);
  await openDeck('heroes');
  await page.locator('input[type=checkbox]').first().dispatchEvent('click');
  await settle();
  check(`${lang} no repeats persisted`, await stored('flashcards:heroes:no-repeats') === true);
  const seen = new Set<number>();
  let repeats = 0;
  let wrongDone = false;
  await page.evaluate(([announce, prompt]: string[]) => {
    (window as any).__samples = [];
    setInterval(() => {
      const samples = (window as any).__samples;
      samples[samples.length] = [document.querySelector(announce)?.textContent ?? '', document.querySelector(`${prompt} img`)?.getAttribute('src') ?? ''];
    }, 50);
  }, [ANNOUNCE, PROMPT]);
  let announceEmptyBefore = true;
  let leaks: string[] = [];
  for (let round = 0; round < heroDeck.length * 2 + 4; round += 1) {
    if (await page.locator(MASTERED).count()) break;
    await page.waitForSelector(OPTION, { state: 'attached', timeout: 10000 });
    const src = await promptSrc();
    const hero: any = heroBySrc.get(src);
    if (!hero) { check(`${lang} oracle knows prompt icon`, false, src); return; }
    const before = await texts(ANNOUNCE);
    if (before.some(Boolean)) announceEmptyBefore = false;
    if (round < 3) leaks = leaks.concat(await attributeLeak(allHeroNames));
    if (seen.has(hero.id)) repeats += 1;
    const index = await correctIndexFor(heroNames(hero));
    if (index < 0) { check(`${lang} correct option present for ${hero.name}`, false, JSON.stringify(await texts(OPTION))); return; }
    if (!wrongDone && round === 1) {
      await clickOption((index + 1) % 4);
      wrongDone = true;
      await sleep(150);
      check(`${lang} wrong answer marks correct and wrong`, await page.locator(`${OPTION}[data-state=wrong]`).count() === 1 && await page.locator(`${OPTION}[data-state=correct]`).count() === 1);
      const text = (await texts(ANNOUNCE)).join(' ');
      check(`${lang} announcement names the answer after reveal`, heroNames(hero).some((name) => text.includes(name)), text);
      await shot(`heroes-wrong-${lang}`);
      await nextCard();
      continue;
    }
    if (round === 2 || round === 3) await page.keyboard.press(String(index + 1));
    else await clickOption(index);
    await sleep(60);
    if (await page.locator(`${OPTION}[data-state=correct]`).count() !== 1) { await sleep(300); }
    seen.add(hero.id);
    if (round === 2) {
      await settle();
      const state = await page.locator(`${OPTION}[data-state=correct]`).count();
      check(`${lang} number key answers`, state === 1, state);
    }
    if (round === 4) {
      const snapshot = await stored('flashcards:heroes:progress');
      await reloadDeck();
      check(`${lang} reload keeps progress`, JSON.stringify(await stored('flashcards:heroes:progress')) === JSON.stringify(snapshot) && (await texts(STAT))[0].startsWith(String(snapshot.stats.correct)), JSON.stringify(snapshot.stats));
      await page.evaluate(([announce, prompt]: string[]) => {
        (window as any).__samples = [];
        setInterval(() => {
          const samples = (window as any).__samples;
          samples[samples.length] = [document.querySelector(announce)?.textContent ?? '', document.querySelector(`${prompt} img`)?.getAttribute('src') ?? ''];
        }, 50);
      }, [ANNOUNCE, PROMPT]);
      continue;
    }
    await nextCard();
  }
  check(`${lang} heroes reach the mastered state`, await page.locator(MASTERED).count() === 1, `seen ${seen.size}/${heroDeck.length}`);
  check(`${lang} no correct hero repeated while no repeats is on`, repeats === 0, repeats);
  check(`${lang} announcement empty before reveal`, announceEmptyBefore);
  check(`${lang} no hero name in attributes before reveal`, leaks.length === 0, leaks.slice(0, 5).join(' | '));
  const samples: [string, string][] = await page.evaluate(() => (window as any).__samples);
  const mismatched = samples.filter(([text, src]) => text && heroBySrc.get(src) && !heroNames(heroBySrc.get(src)).some((name) => text.includes(name)));
  check(`${lang} polled announcement never shows another card's answer`, mismatched.length === 0 && samples.some(([text]) => text), `${samples.length} samples, ${mismatched.length} mismatched ${JSON.stringify(mismatched.slice(0, 2))}`);
  await shot(`heroes-mastered-${lang}`);
  const stats = await stored('flashcards:heroes:progress');
  note(`${lang} final stats ${JSON.stringify(stats.stats)} seen ${stats.seenIds.length}/${heroDeck.length}`);
}

async function playToMastered(deck: string, lang: Lang, ids: number[]) {
  await reset(lang, 'english', nearEnd(deck, ids));
  await openDeck(deck);
  await shot(`${deck}-start-${lang}`);
  for (let round = 0; round < 40; round += 1) {
    if (await page.locator(MASTERED).count()) break;
    await clickOption(0);
    await sleep(120);
    if (round === 0) await shot(`${deck}-revealed-${lang}`);
    await nextCard();
  }
  check(`${lang} ${deck} seeded near end reaches mastered`, await page.locator(MASTERED).count() === 1);
  await shot(`${deck}-mastered-${lang}`);
}

async function decks() {
  for (const lang of ['de', 'en'] as Lang[]) {
    await playToMastered('items', lang, itemDeck.slice(1).map((item: any) => item.id));
    await playToMastered('item-upgrades', lang, upgradeDeck.slice(1).map((entry: any) => entry.id));
    await reset(lang);
    await openDeck('items');
    const src = await promptSrc();
    const item: any = itemBySrc.get(src);
    check(`${lang} items prompt icon from the pool`, Boolean(item), src);
    const names = [item?.name, (itemDe.get(item?.id) as any)?.name].filter(Boolean) as string[];
    const leak = await attributeLeak(names);
    check(`${lang} no item name in attributes before reveal`, leak.length === 0, leak.join('|'));
    await reset(lang);
    await openDeck('item-upgrades');
    await shot(`item-upgrades-idle-${lang}`);
    check(`${lang} upgrades deck shows 4 options`, await page.locator(OPTION).count() === 4);
  }
}

function longestEffectIds(count: number) {
  return [...effectDeck].sort((a: any, b: any) => JSON.stringify(effectSections(b, anyTerms)).length - JSON.stringify(effectSections(a, anyTerms)).length).slice(0, count).map((item: any) => item.id);
}

async function effects() {
  for (const lang of ['de', 'en'] as Lang[]) {
    await playToMastered('item-effects', lang, effectDeck.slice(1).map((item: any) => item.id));
    await reset(lang);
    await openDeck('item-effects');
    let exposed = 0;
    for (let round = 0; round < 8; round += 1) {
      const promptText = (await texts(PROMPT)).join(' ');
      await clickOption(0);
      await sleep(150);
      const answer = (await texts(`${OPTION}[data-state=correct]`))[0] ?? '';
      if (answer && promptText.includes(answer.replace(/^\d/, '').trim())) { exposed += 1; note(`${lang} effect prompt contains its name: ${answer}`); }
      await nextCard();
    }
    check(`${lang} effects to name prompt never contains the full answer name`, exposed === 0, exposed);
    const direction = page.locator('[role=group] button[aria-pressed=false]').last();
    await direction.dispatchEvent('click');
    await settle();
    await shot(`item-effects-reverse-${lang}`);
    const legendary = page.locator('label input[type=checkbox]').nth(1);
    await legendary.dispatchEvent('click');
    await settle();
    let tierFive = 0;
    const named = (name: string) => [...itemDeck, ...itemsDe].find((item: any) => item.name === name);
    for (let round = 0; round < 15; round += 1) {
      const prompt = await promptSrc();
      const item: any = itemBySrc.get(prompt);
      if (item?.item_tier === 5) tierFive += 1;
      await clickOption(0);
      await sleep(120);
      await nextCard();
    }
    check(`${lang} exclude legendary keeps tier 5 out of the prompts`, tierFive === 0, `${tierFive} ${typeof named}`);
    const longest = longestEffectIds(1)[0];
    await reset(lang, 'english', nearEnd('item-effects', effectDeck.filter((item: any) => item.id !== longest).map((item: any) => item.id)));
    await openDeck('item-effects');
    await page.locator('[role=group] button[aria-pressed=false]').last().dispatchEvent('click');
    await settle();
    const maxLength = Math.max(...(await texts(OPTION)).map((text: string) => text.length));
    note(`${lang} longest option rendered ${maxLength} chars`);
    await shot(`item-effects-longest-${lang}`);
  }
}

async function edge() {
  await reset('de', 'german');
  await openDeck('heroes');
  await clickOption(0);
  await sleep(150);
  await nextCard();
  await page.locator('button[aria-expanded]').first().dispatchEvent('click');
  await settle();
  await shot('reset-confirm-de');
  await page.getByText('Abbrechen', { exact: true }).first().dispatchEvent('click');
  await settle();
  check('reset cancel keeps progress', (await stored('flashcards:heroes:progress')).stats.seen === 1);
  await page.locator('button[aria-expanded]').first().dispatchEvent('click');
  await settle();
  await page.locator('button', { hasText: 'Zurücksetzen' }).last().dispatchEvent('click');
  await settle();
  check('reset confirm clears progress', (await stored('flashcards:heroes:progress'))?.stats?.seen === 0);

  const src = await promptSrc();
  const germanOptions = await texts(OPTION);
  await switchTerms('english');
  check('terms switch keeps the card', await promptSrc() === src);
  const englishOptions = await texts(OPTION);
  note(`options german ${germanOptions.join(',')} english ${englishOptions.join(',')}`);
  check('terms switch keeps option order by hero', englishOptions.length === germanOptions.length);
  await switchLang('en');
  check('dashboard language keeps the card', await promptSrc() === src);
  check('dashboard language keeps option names', JSON.stringify(await texts(OPTION)) === JSON.stringify(englishOptions));
  check('dashboard language changes UI', /accuracy/i.test(await pageText()));

  await reset('de', 'english', { 'flashcards:heroes:progress': { stats: { correct: 9, seen: 1 }, seenIds: 'x' }, 'flashcards:heroes:no-repeats': 'ja' });
  await openDeck('heroes');
  check('corrupt progress falls back to fresh', (await texts(STAT))[0].startsWith('0'));
  check('corrupt no repeats falls back to off', !(await page.locator('input[type=checkbox]').first().isChecked()));
  await page.evaluate(() => localStorage.setItem('flashcards:heroes:progress', '{not json'));
  await reloadDeck();
  check('unparsable progress falls back to fresh', (await texts(STAT))[0].startsWith('0'));
  await reset('de', 'german');
  await openDeck('items');
  const itemSrc = await promptSrc();
  const germanItems = await texts(OPTION);
  const statBefore = (await texts(STAT)).join('|');
  await switchTerms('english');
  const englishItems = await texts(OPTION);
  note(`item options german ${germanItems.join(',')} english ${englishItems.join(',')}`);
  const translated = germanItems.every((name: string, index: number) => {
    const shown = name.replace(/^\d/, '').trim();
    const german: any = itemsDe.find((item: any) => item.name === shown);
    const english: any = itemsEn.find((item: any) => item.id === german?.id);
    return english && englishItems[index].includes(english.name);
  });
  check('terms switch relocalizes item names on the same card', await promptSrc() === itemSrc && translated, englishItems.join(','));
  check('terms switch keeps progress', (await texts(STAT)).join('|') === statBefore, statBefore);
  await page.goto(`${base}/mini-spiele?spiel=flashcards&stapel=gibtsnicht`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="stapel="]', { state: 'attached', timeout: 60000 });
  check('unknown stapel shows the deck hub', await page.locator('a[href*="stapel="]').count() === 4);
}

async function openResult(query: string) {
  await page.goto(`${base}/mini-spiele/ergebnis${query}`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('[data-testid=result-row], [data-testid=result-invalid]', { state: 'attached', timeout: 30000 });
  await sleep(150);
}

async function result() {
  const date = dayBefore(1);
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.evaluate(() => localStorage.setItem('dashboard.language.v2', 'de'));
  const started = requests.length;
  await openResult(`?d=${date}&r=hw3-6.il5-5.sw1-4.vw12-20.tw8-10`);
  const rows = await page.evaluate(() => [...document.querySelectorAll('[data-testid=result-row]')].map((row) => [row.getAttribute('data-mode'), row.getAttribute('data-state'), (row as HTMLElement).innerText.replace(/\s+/g, ' ')]));
  note(JSON.stringify(rows));
  check('result lists every game', rows.length === 7);
  check('result row states', rows.filter((row: any) => row[1] === 'won').length === 4 && rows.filter((row: any) => row[1] === 'lost').length === 1 && rows.filter((row: any) => row[1] === 'open').length === 2);
  check('result day number', (await texts('[data-testid=result-day]'))[0].includes(String(getDayNumber(date))), (await texts('[data-testid=result-day]'))[0]);
  note((await texts('[data-testid=result-date]'))[0]);
  check('result summary', (await texts('[data-testid=result-summary]'))[0] === 'Gewonnen: 4, verloren: 1, nicht gespielt: 2', (await texts('[data-testid=result-summary]'))[0]);
  const href = await page.locator('[data-testid=result-play]').getAttribute('href');
  check('play link carries the date', href === `/mini-spiele?date=${date}`, href);
  check('standalone without the dashboard shell', await page.locator('nav').count() === 0 && await page.locator('[data-language-switch]').count() === 0);
  await shot('result-de');
  const apiCalls = requests.slice(started).filter((url) => /\/api\/|\/twitch\/api|deadlock-api/.test(url));
  check('result page makes no data requests', apiCalls.length === 0, apiCalls.slice(0, 4).join(' '));
  await page.evaluate(() => localStorage.setItem('dashboard.language.v2', 'en'));
  await openResult(`?d=${date}&r=hw3-6.il5-5.sw1-4.vw12-20.tw8-10`);
  check('result summary in english', (await texts('[data-testid=result-summary]'))[0] === 'Won: 4, lost: 1, not played: 2', (await texts('[data-testid=result-summary]'))[0]);
  await shot('result-en');
  await openResult(`?d=${date}&r=hw3-6.il5-5.sw1-4.al5-5.vl3-20.tl4-10.ow10-10`);
  check('all games played', await page.locator('[data-testid=result-row][data-state=open]').count() === 0);
  await shot('result-full-en');
  const invalid = ['', `?d=${date}`, '?r=hw3-6', `?d=&r=hw3-6`, `?d=${date}&r=`, '?d=2026-02-30&r=hw3-6', '?d=2026-03-21&r=hw3-6', '?d=2999-01-01&r=hw3-6', `?d=${date}&d=${dayBefore(2)}&r=hw3-6`, `?d=${date}&r=hw3-6&r=hw3-6`, `?d=${date}&r=xw3-6`, `?d=${date}&r=hw7-6`, `?d=${date}&r=tw11-10`, `?d=${date}&r=${'hw3-6.'.repeat(30)}hw3-6`, `?d=${date}&r=%3Cscript%3E`, `?d=${date}&r=hw3-6.hw3-6`, `?d=null&r=null`];
  let bad = 0;
  for (const query of invalid) {
    await openResult(query);
    if (await page.locator('[data-testid=result-invalid]').count() !== 1) { bad += 1; note(`not invalid: ${query}`); }
  }
  check(`all ${invalid.length} bad links give the invalid state`, bad === 0, bad);
  check('invalid state wording in english', /This link is invalid/.test(await pageText()));
  await page.evaluate(() => localStorage.setItem('dashboard.language.v2', 'de'));
  await openResult('?d=kaputt&r=hw3-6');
  check('invalid state wording in german', /Dieser Link ist ungültig/.test(await pageText()));
  await shot('result-invalid-de');
}

const finishedSeed = (date: string) => ({
  [`deadlockdle:guess-hero:game:${date}`]: { date, status: 'won', guesses: [1, 2, 3], hintsRevealed: 2 },
  [`deadlockdle:guess-item:game:${date}`]: { date, status: 'lost', guesses: [1, 2, 3, 4, 5], hintsRevealed: 4 },
  [`deadlockdle:guess-sound:game:${date}`]: { date, status: 'won', guesses: [1], hintsRevealed: 0 },
  [`deadlockdle:guess-ability:game:${date}`]: { date, status: 'won', guesses: [1, 2], hintsRevealed: 1 },
  [`deadlockdle:item-stats:game:${date}`]: { date, answers: {}, submitted: true, score: 12, totalFields: 20 },
  [`deadlockdle:trivia:game:${date}`]: { date, currentQuestion: 10, answers: Array.from({ length: 10 }, () => 0), score: 8, completed: true },
  [`deadlockdle:higher-lower:game:${date}`]: { date, currentQuestion: 10, answers: Array.from({ length: 10 }, () => 'higher'), score: 4, completed: true },
});
const EXPECTED = 'hw3-6.il5-5.sw1-4.aw2-5.vw12-20.tw8-10.ol4-10';
const stubClipboard = () => page.evaluate(() => {
  (window as any).__copied = [];
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (value: string) => { const list = (window as any).__copied; list[list.length] = value; } } });
});
const lastCopied = () => page.evaluate(() => (window as any).__copied.at(-1));

async function clickShare(label: RegExp) {
  await stubClipboard();
  await page.getByRole('button', { name: label }).first().dispatchEvent('click');
  await sleep(250);
  return lastCopied();
}

async function share() {
  await reset('de', 'english', finishedSeed(today));
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="spiel="]', { state: 'attached', timeout: 60000 });
  await settle();
  const link = `https://dead-lock.de/twitch/mini-spiele/ergebnis?d=${today}&r=${EXPECTED}`;
  const fromHub = await clickShare(/Alle Ergebnisse teilen/);
  check('hub button copies the overview link', fromHub === link, fromHub);
  check('confirmation is Link kopiert', /Link kopiert/.test(await pageText()));
  await shot('share-confirm-de');
  for (const [mode, selector] of [['trivia', '[data-testid=game-result]'], ['guess-hero', '[data-testid=game-result]']] as const) {
    await page.goto(`${base}/mini-spiele?spiel=${mode}`, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector(selector, { state: 'attached', timeout: 90000 }).catch(() => undefined);
    await settle();
    const copied = await clickShare(/Ergebnis teilen/).catch((error: Error) => `error ${error.message}`);
    check(`${mode} button copies the same link`, copied === link, copied);
  }
  await switchLang('en').catch(() => undefined);
  await settle();
  await stubClipboard();
  await page.getByRole('button', { name: /Share result/ }).first().dispatchEvent('click').catch(() => undefined);
  await sleep(250);
  check('confirmation is Link copied', /Link copied/.test(await pageText()));
  const query = new URL(link).search;
  await openResult(query);
  check('copied link opens as a valid overview', await page.locator('[data-testid=result-row]').count() === 7 && await page.locator('[data-testid=result-row][data-state=open]').count() === 0);
  const body = await pageText();
  check('overview shows no answers', !allHeroNames.some((name) => body.includes(name)));
  await reset('de', 'english', { [`deadlockdle:guess-hero:game:${today}`]: { date: today, status: 'won', guesses: [1], hintsRevealed: 0 }, [`deadlockdle:trivia:game:${today}`]: { date: today, currentQuestion: 'x' } });
  await page.goto(`${base}/mini-spiele?spiel=guess-hero`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('[data-testid=game-result]', { state: 'attached', timeout: 90000 }).catch(() => undefined);
  await settle();
  const partial = await clickShare(/Ergebnis teilen/).catch((error: Error) => `error ${error.message}`);
  check('partial day link drops corrupt and unplayed games', partial === `https://dead-lock.de/twitch/mini-spiele/ergebnis?d=${today}&r=hw1-6`, partial);
}

const HINT_MODES = ['guess-hero', 'guess-item', 'guess-sound', 'guess-ability'] as const;
const hintLayout = () => page.evaluate(() => [...document.querySelectorAll('dl dt')].map((dt) => {
  const dd = dt.nextElementSibling as HTMLElement;
  const text = dd.querySelector('span:not(.invisible)') ?? dd;
  const range = document.createRange();
  range.selectNodeContents(dt);
  const label = range.getBoundingClientRect();
  range.selectNodeContents(text);
  const value = range.getBoundingClientRect();
  const apart = label.right <= value.left + 0.5 || label.bottom <= value.top + 0.5;
  return { label: dt.textContent ?? '', value: dd.textContent ?? '', apart, gap: Math.round((value.left - label.right) * 10) / 10, top: Math.round(label.top), labelWidth: Math.round(label.width) };
}));
async function hintPage(mode: string, date: string) {
  await page.goto(`${base}/mini-spiele?spiel=${mode}&date=${date}`, { waitUntil: 'domcontentloaded' });
  const found = await page.waitForSelector('dl dt', { state: 'attached', timeout: 45000 }).then(() => true).catch(() => false);
  if (!found) check(`hint panel loads for ${mode} ${date}`, false, (await pageText().catch(() => 'no text')).replace(/\s+/g, ' ').slice(-250));
  await sleep(450);
}
async function hints() {
  const dates = [today, dayBefore(1), dayBefore(2), dayBefore(3), dayBefore(4), dayBefore(5)];
  for (const lang of ['de', 'en'] as Lang[]) {
    const labels = new Set<string>();
    let overlapping = 0;
    for (const mode of HINT_MODES) {
      await reset(lang, 'german', Object.fromEntries(dates.map((date) => [`deadlockdle:${mode}:game:${date}`, { date, status: 'playing', guesses: [], hintsRevealed: 9 }])));
      for (const date of dates) {
        await hintPage(mode, date);
        const layout = await hintLayout();
        for (const row of layout) {
          labels.add(`${mode}:${row.label}`);
          if (!row.apart) { overlapping += 1; note(`overlap ${lang} ${mode} ${date} ${JSON.stringify(row)}`); }
        }
        if (date === today) { await shot(`hints-${mode}-${lang}`); note(`${lang} ${mode} ${JSON.stringify(layout)}`); }
      }
    }
    note(`${lang} labels measured: ${[...labels].join(', ')}`);
    check(`${lang} no hint label overlaps its value`, overlapping === 0, overlapping);
  }
}

async function hintValues(mode: string, lang: Lang, terms: 'english' | 'german', dates: string[]) {
  await reset(lang, terms, Object.fromEntries(dates.map((date) => [`deadlockdle:${mode}:game:${date}`, { date, status: 'playing', guesses: [], hintsRevealed: 9 }])));
  const values: string[] = [];
  for (const date of dates) {
    await hintPage(mode, date);
    for (const row of await hintLayout()) values[values.length] = `${row.label}=${row.value}`;
  }
  return values;
}
async function termsCheck() {
  const dates = Array.from({ length: 8 }, (_, index) => dayBefore(index));
  const GERMAN = /=(Waffe|Vitalität|Geist|Aktiv|Passiv|Signaturfähigkeit|Ultimative Fähigkeit|Angeborene Fähigkeit|Assassine|Schläger|Schütze|Mystiker)$/;
  const ENGLISH = /=(Weapon|Vitality|Spirit|Active|Passive|Signature|Ultimate|Innate)$/;
  const TYPES = /=(Assassin|Brawler|Marksman|Mystic)$/;
  for (const mode of ['guess-hero', 'guess-item', 'guess-ability', 'guess-sound']) {
    const deEnglish = await hintValues(mode, 'de', 'english', dates);
    const deGerman = await hintValues(mode, 'de', 'german', dates);
    const enGerman = await hintValues(mode, 'en', 'german', dates);
    const termRows = (rows: string[], pattern: RegExp) => rows.filter((row) => pattern.test(row));
    note(`${mode} de+english ${[...new Set(termRows(deEnglish, ENGLISH).concat(termRows(deEnglish, TYPES)))].join(', ')}`);
    note(`${mode} de+german ${[...new Set(termRows(deGerman, GERMAN).concat(termRows(deGerman, TYPES)))].join(', ')}`);
    note(`${mode} en+german ${[...new Set(termRows(enGerman, GERMAN).concat(termRows(enGerman, TYPES)))].join(', ')}`);
    check(`${mode} English terms on German dashboard show no German term`, termRows(deEnglish, GERMAN).length === 0, termRows(deEnglish, GERMAN).join('|'));
    check(`${mode} German terms show no English term`, termRows(deGerman, ENGLISH).length === 0 && termRows(enGerman, ENGLISH).length === 0, termRows(deGerman, ENGLISH).concat(termRows(enGerman, ENGLISH)).join('|'));
    check(`${mode} hero types stay English`, ![...deEnglish, ...deGerman, ...enGerman].some((row) => /=(Assassine|Schläger|Schütze|Mystiker)$/.test(row)));
    if (mode === 'guess-hero' || mode === 'guess-sound') check(`${mode} hero type values seen`, [...deGerman, ...deEnglish].some((row) => TYPES.test(row)));
    if (mode !== 'guess-hero' && mode !== 'guess-sound') check(`${mode} term values seen`, termRows(deEnglish, ENGLISH).length > 0 && termRows(deGerman, GERMAN).length > 0);
    const dtEnglish = deEnglish[0].split('=')[0];
    const dtGerman = enGerman[0].split('=')[0];
    note(`${mode} label follows dashboard: de ${dtEnglish} en ${dtGerman}`);
    check(`${mode} labels follow the dashboard language`, dtEnglish !== dtGerman);
  }
  await reset('de', 'english');
  await page.goto(`${base}/mini-spiele?spiel=item-stats`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector(OPTION, { state: 'attached', timeout: 90000 });
  await settle();
  let body = await pageText();
  check('item stats tiles English terms on German dashboard', /Weapon/.test(body) && /Vitality/.test(body) && /Spirit/.test(body) && /Active/.test(body) && /Passive/.test(body) && /Kategorie|KATEGORIE/.test(body) && !/\bWaffe\b|Vitalität|\bPassiv\b/.test(body));
  await shot('terms-item-stats-de-english');
  await switchTerms('german');
  body = await pageText();
  check('item stats tiles German terms', /Waffe/.test(body) && /Vitalität/.test(body) && /Geist/.test(body) && /Aktiv/.test(body) && /Passiv/.test(body));
}

async function tiles() {
  for (const lang of ['de', 'en'] as Lang[]) {
    for (const terms of ['german', 'english'] as const) {
      await reset(lang, terms, { [`deadlockdle:item-stats:game:${today}`]: { date: today, answers: {}, submitted: true, score: 12, totalFields: 20 } });
      await page.goto(`${base}/mini-spiele?spiel=item-stats`, { waitUntil: 'domcontentloaded' });
      await page.waitForSelector(`${OPTION}[data-state=correct]`, { state: 'attached', timeout: 90000 });
      await sleep(500);
      const report = await page.evaluate((selector: string) => {
        const rows: string[] = [];
        let marks = 0;
        for (const tile of document.querySelectorAll(selector)) {
          const mark = tile.querySelector('svg');
          const textNode = [...tile.childNodes].find((node) => node.nodeType === Node.TEXT_NODE);
          if (!textNode) continue;
          const range = document.createRange();
          range.selectNode(textNode);
          const text = range.getBoundingClientRect();
          const box = tile.getBoundingClientRect();
          if (text.left < box.left - 0.5 || text.right > box.right + 0.5) rows[rows.length] = `clipped ${tile.textContent} ${Math.round(text.width)}/${Math.round(box.width)}`;
          if (!mark) continue;
          marks += 1;
          const icon = mark.getBoundingClientRect();
          const clear = text.right <= icon.left + 0.5 || text.left >= icon.right - 0.5 || text.bottom <= icon.top + 0.5 || text.top >= icon.bottom - 0.5;
          if (!clear) rows[rows.length] = `overlap ${tile.textContent} text ${Math.round(text.right)} mark ${Math.round(icon.left)}`;
        }
        return { rows, marks };
      }, OPTION);
      check(`${lang} ${terms} result tiles: no mark over text`, report.rows.length === 0 && report.marks > 0, JSON.stringify(report));
      await shot(`item-stats-result-${lang}-${terms}`);
    }
  }
}

if (sections.has('hub')) await hub();
if (sections.has('heroes')) { await heroes('de'); if (WIDTH === 390) await heroes('en'); }
if (sections.has('decks')) await decks();
if (sections.has('effects')) await effects();
if (sections.has('edge')) await edge();
if (sections.has('result')) await result();
if (sections.has('share')) await share();
if (sections.has('hints')) await hints();
if (sections.has('terms')) await termsCheck();
if (sections.has('tiles')) await tiles();

const hosts = [...new Set(requests.map((url) => { try { return new URL(url).host; } catch { return url.slice(0, 20); } }))];
note(`hosts ${hosts.join(', ')}; aborted ${aborted}`);
check('only allowed hosts', hosts.every((host) => /^(127\.0\.0\.1(:\d+)?|assets(-bucket)?\.deadlock-api\.com|api\.deadlock-api\.com)$/.test(host) || host === ''), hosts.join(','));
check('no page or console errors', problems.length === 0, problems.slice(0, 6).join(' | '));
console.log(`failures: ${failures}`);
await page.close();
await browser.close();
process.exit(0);
