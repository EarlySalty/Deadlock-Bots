const W = '/home/nathanael/.worktrees/tb-mini-spiele-p2/bot/dashboard_v2/';
const L = W + 'src/minigames/lib/';
const { chromium } = await import(W + 'node_modules/playwright-core/index.mjs');
const { displayName, fetchApiList, fetchApiJson } = await import(L + 'api.ts');
const { generateDailyQuestions, buildAbilitiesWithHeroes, hasDisplayName } = await import(L + 'deadlockdle/trivia-questions.ts');
const { heroPoolOn, itemPoolOn } = await import(L + 'deadlockdle/roster.ts');
const { getModeSeed, getTodayDate, seededPick, seededRandom, seededShuffle } = await import(L + 'deadlockdle/seed.ts');
const { gameStorageKey, streakStorageKey } = await import(L + 'deadlockdle/storage.ts');
const { buildGuessableAbilities } = await import(L + 'deadlockdle/hints.ts');
const { itemStatQuestion } = await import(L + 'deadlockdle/item-stat.ts');
const { extractSounds, guessableAbilities } = await import(L + 'deadlockdle/sound-pool.ts');
const { STATS_RANKS, buildRounds, correctGuess, formatValue, pickAbilityHeroes, statsWindow } = await import(L + 'deadlockdle/higher-lower.ts');

const [moliPort, base, shotDir, runKind, sectionList = 'hub,terms,trivia,higher,items,sound'] = process.argv.slice(2);
const sections = new Set(sectionList.split(','));
const today = getTodayDate();
const yesterday = new Date(Date.parse(`${today}T00:00:00Z`) - 86400000).toISOString().slice(0, 10);
const LABEL = { de: 'Deutsch', en: 'English' } as const;
const LOCALE = { de: 'de-DE', en: 'en-US' } as const;
const ROUND_TYPES = new Set(['signature', 'ultimate']);
type Lang = 'de' | 'en';

const clean = (list: any[]) => list.map((entry) => ({ ...entry, name: displayName(entry.name) }));
const assets = async (path: string, params: Record<string, string>) => clean(await fetchApiList(path, params));
async function languageData(language: string) {
  return {
    heroes: await assets('/v1/assets/heroes', { only_active: 'false', language }),
    items: await assets('/v1/assets/items/by-type/upgrade', { language }),
    abilities: await assets('/v1/assets/items/by-type/ability', { language }),
  };
}

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${moliPort}`);
const context = browser.contexts()[0] ?? (await browser.newContext());
const page = await context.newPage();
const WIDTH = Number(process.env.P2_WIDTH ?? 1440);
await page.setViewportSize({ width: WIDTH, height: WIDTH < 500 ? 844 : 900 });
if (WIDTH < 640) await page.addInitScript(() => { addEventListener('DOMContentLoaded', () => { const tag = document.createElement('style'); tag.textContent = 'dd > span.invisible{display:none!important}'; document.head.appendChild(tag); }); });
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

const out: string[] = [];
const note = (line: string) => { out[out.length] = `    ${line}`; console.log(`    ${line}`); };
const check = (name: string, ok: boolean, detail: unknown = '') => { const line = `${ok ? 'ok  ' : 'FAIL'} ${name}${detail !== '' ? ` [${String(detail).slice(0, 300)}]` : ''}`; out[out.length] = line; console.log(line); };
const sleep = (ms: number) => page.waitForTimeout(ms);
const soundRequests = () => requests.filter((url) => url.includes('/v1/assets/sounds')).length;

async function shot(name: string, width: number) {
  if (width !== WIDTH) return;
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
    const wide: string[] = [];
    for (const element of document.querySelectorAll('body *')) {
      const rect = element.getBoundingClientRect();
      if (rect.right > limit + 1 && rect.width > 0 && !element.closest('nav') && wide.length < 8 && (element.parentElement?.getBoundingClientRect().right ?? 0) <= limit + 1) wide[wide.length] = `${element.tagName}.${String(element.getAttribute('class')).slice(0, 50)}:${Math.round(rect.left)}-${Math.round(rect.right)}`;
    }
    const chain: string[] = [];
    for (const element of document.querySelectorAll('body *')) {
      if (element.scrollWidth > element.clientWidth + 1 && element.clientWidth > 300 && chain.length < 6) chain[chain.length] = `${element.tagName}.${String(element.getAttribute('class')).slice(0, 40)} ${element.clientWidth}/${element.scrollWidth} ${getComputedStyle(element).overflowX}`;
    }
    const active = document.activeElement as HTMLElement | null;
    const look = active ? getComputedStyle(active) : null;
    return { width: limit, scroll: document.documentElement.scrollWidth, offenders, wide, chain, focus: active ? `${active.tagName}.${String(active.getAttribute('class')).slice(0, 40)} outline ${look?.outlineStyle} ${look?.outlineWidth}` : '' };
  });
  check(`${name} no horizontal overflow at ${width}`, overflow.offenders.length === 0 && overflow.scroll <= overflow.width, JSON.stringify(overflow));
  if (process.env.P2_NOSHOT) return;
  const taken = page.screenshot({ path: `${shotDir}/p2-${name}-${width}.png`, fullPage: true, timeout: 25000 }).then(() => true).catch(() => false);
  const ok = await Promise.race([taken, new Promise((done) => setTimeout(() => done(false), 30000))]);
  if (!ok) note(`screenshot failed ${name} ${width}`);
}
async function reset(lang: Lang, seedStreakMode?: string) {
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="spiel="]', { state: 'attached', timeout: 60000 });
  await page.evaluate(([language, key, last]: string[]) => {
    for (const name of Object.keys(localStorage)) if (name.startsWith('deadlockdle:')) localStorage.removeItem(name);
    localStorage.setItem('dashboard.language.v2', language);
    if (key) localStorage.setItem(key, JSON.stringify({ currentStreak: 3, maxStreak: 3, lastPlayedDate: last, gamesPlayed: 3, gamesWon: 3 }));
  }, [lang, seedStreakMode ? streakStorageKey(seedStreakMode) : '', yesterday]);
}
async function openMode(mode: string, selector: string) {
  await page.goto(`${base}/mini-spiele?spiel=${mode}`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector(selector, { state: 'attached', timeout: 90000 });
}
async function reload(selector: string) {
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector(selector, { state: 'attached', timeout: 90000 });
}
async function switchLang(lang: Lang) {
  await page.locator(`[data-language-switch] button[title="${LABEL[lang]}"]`).first().dispatchEvent('click');
  await sleep(400);
}
async function switchTerms(value: 'english' | 'german') {
  await page.locator(`[data-game-terms-switch] button[data-value=${value}]`).first().dispatchEvent('click');
  await sleep(500);
}
const pageText = () => page.evaluate(() => (document.querySelector('main') ?? document.body as HTMLElement).innerText);
const pageContent = () => page.evaluate(() => document.querySelector('main')?.textContent ?? '');
const stored = (key: string) => page.evaluate((name: string) => JSON.parse(localStorage.getItem(name) ?? 'null'), key);
const attributeLeak = (needles: string[]) => page.evaluate((list: string[]) => {
  const found: string[] = [];
  for (const element of document.querySelectorAll('main *')) {
    for (const name of ['alt', 'title', 'aria-label']) {
      const value = element.getAttribute(name) ?? '';
      if (value && list.some((needle) => value.includes(needle))) found[found.length] = `${name}=${value}`;
    }
  }
  return found;
}, needles);
const dispatch = (selector: string) => page.locator(selector).first().dispatchEvent('click');
const QUESTION = 'p[tabindex="-1"]';
const OPTION = '[data-answer-option]';
const RESULT = '[data-testid=game-result]';

async function quizStep(index: number, optionIndex: number, key: string | null) {
  await page.waitForSelector(QUESTION, { state: 'attached', timeout: 10000 });
  await page.waitForFunction((value: number) => [...document.querySelectorAll('p')].some((p) => p.textContent?.includes(`${value}/10`)), index + 1, { timeout: 10000 });
  if (key) await page.keyboard.press(key);
  else await page.locator(OPTION).nth(optionIndex).dispatchEvent('click');
  await page.waitForSelector('[data-answer-option][data-state=correct]', { state: 'attached', timeout: 5000 });
}
const nextQuestion = () => dispatch('[data-state=revealed] button');

async function playTrivia(lang: Lang, questions: any[], wrongAt: Set<number>, staleCheck: boolean) {
  const other: Lang = lang === 'de' ? 'en' : 'de';
  const key = gameStorageKey('trivia', today);
  const streakKey = streakStorageKey('trivia');
  let expected = 0;
  await reset(lang, wrongAt.size === 0 ? 'trivia' : undefined);
  await openMode('trivia', OPTION);
  let start = 0;
  if (staleCheck) {
    const truth = questions[0].correctIndex;
    await page.evaluate(([name, value, day]: [string, number, string]) => {
      const state = JSON.parse(localStorage.getItem(name) ?? 'null') ?? { date: day, currentQuestion: 0, answers: Array.from({ length: 10 }, () => null), score: 0, completed: false };
      state.answers[0] = value; state.currentQuestion = 1; state.score = 1;
      localStorage.setItem(name, JSON.stringify(state));
    }, [key, truth, today]);
    await page.locator(OPTION).nth((truth + 1) % questions[0].options.length).dispatchEvent('click');
    await sleep(300);
    const after = await stored(key);
    check(`${lang} trivia stale tab does not overwrite stored answer`, after?.answers?.[0] === truth && after?.score === 1, JSON.stringify(after).slice(0, 80));
    await reload(OPTION);
    start = 1;
    expected = 1;
  }
  for (let index = start; index < questions.length; index += 1) {
    const question = questions[index];
    await page.waitForFunction((value: number) => [...document.querySelectorAll('p')].some((p) => p.textContent?.includes(`${value}/10`)), index + 1, { timeout: 10000 });
    await sleep(120);
    const leakBefore = await page.locator('[data-answer-option][data-state=correct], [data-answer-option][data-state=wrong]').count();
    if (leakBefore !== 0) check(`${lang} trivia q${index} marks before answer`, false, leakBefore);
    const wrong = wrongAt.has(index);
    const option = wrong ? (question.correctIndex + 1) % question.options.length : question.correctIndex;
    if (!wrong) expected += 1;
    const useKey = index === 2 ? String(option + 1) : null;
    const prompt = await page.locator(QUESTION).first().textContent();
    await quizStep(index, option, useKey);
    const correctAt = await page.locator(OPTION).evaluateAll((list: Element[]) => list.findIndex((element) => element.getAttribute('data-state') === 'correct'));
    check(`${lang} trivia q${index + 1} reveal matches generator`, correctAt === question.correctIndex, `${prompt}${useKey ? ' (key)' : ''}`);
    if (index === 0) await shot(`trivia-${lang}-revealed`, 1440);
    if (index === 1) await shot(`trivia-${lang}-revealed`, 390);
    if (index === 3) {
      await reload(OPTION);
      const text = await pageText();
      check(`${lang} trivia reload resumes at next question`, text.includes('5/10'));
      const before = await page.locator(QUESTION).first().textContent();
      await switchLang(other);
      const switched = await page.locator(QUESTION).first().textContent();
      check(`${lang} trivia language switch relocalizes mid game`, before !== switched, `${before} => ${switched}`);
      await switchLang(lang);
      continue;
    }
    await nextQuestion();
  }
  await page.waitForSelector(RESULT, { state: 'attached', timeout: 10000 });
  const score = await page.locator('[data-testid=quiz-score]').textContent();
  check(`${lang} trivia final score`, score === `${expected}/10`, score);
  const streak = await stored(streakKey);
  check(`${lang} trivia day streak`, streak?.currentStreak === (wrongAt.size === 0 ? 4 : 1), JSON.stringify(streak));
  await sleep(600);
  await shot(`trivia-${lang}-result`, 1440);
  await shot(`trivia-${lang}-result`, 390);
  await reload(RESULT);
  check(`${lang} trivia result survives reload`, (await page.locator('[data-testid=quiz-score]').textContent()) === score);
  note(`${lang} trivia real result ${score}, streak ${streak?.currentStreak}`);
}

async function playHigherLower(lang: Lang, rounds: any[], wrongAt: Set<number>) {
  const other: Lang = lang === 'de' ? 'en' : 'de';
  const streakKey = streakStorageKey('higher-lower');
  let expected = 0;
  await reset(lang, wrongAt.size === 0 ? 'higher-lower' : undefined);
  await openMode('higher-lower', OPTION);
  for (let index = 0; index < rounds.length; index += 1) {
    const round = rounds[index];
    const answer = correctGuess(round);
    const wrong = wrongAt.has(index);
    const guess = wrong ? (answer === 'higher' ? 'lower' : 'higher') : answer;
    if (!wrong) expected += 1;
    const position = guess === 'lower' ? 0 : 1;
    const hiddenValue = formatValue(round.right.value, round.format, LOCALE[lang]);
    await page.waitForFunction((value: number) => [...document.querySelectorAll('p')].some((p) => p.textContent?.includes(`${value}/10`)), index + 1, { timeout: 10000 });
    await sleep(450);
    const before = await pageContent();
    const hiddenCount = await page.locator('p[data-state=hidden]').count();
    check(`${lang} higher-lower r${index + 1} value hidden before answer`, hiddenCount === (round.kind === 'head-to-head' ? 2 : 1) && !before.includes(hiddenValue) && before.includes('/10') && (await attributeLeak([hiddenValue])).length === 0, `${round.kind} ${hiddenValue}`);
    let key: string | null = null;
    if (index === 0 || index === 2) key = round.kind === 'head-to-head' ? String(position + 1) : guess === 'higher' ? 'h' : lang === 'de' ? 'n' : 'l';
    await quizStep(index, position, key);
    await sleep(450);
    const after = await pageContent();
    const correctAt = await page.locator(OPTION).evaluateAll((list: Element[]) => list.findIndex((element) => element.getAttribute('data-state') === 'correct'));
    check(`${lang} higher-lower r${index + 1} reveal shows value and right answer`, after.includes(hiddenValue) && correctAt === (answer === 'lower' ? 0 : 1), `${key ? `key ${key} ` : ''}${hiddenValue} shown=${after.includes(hiddenValue)} correctAt=${correctAt}`);
    if (index === 0) await shot(`higher-lower-${lang}-revealed`, 1440);
    if (index === 1) await shot(`higher-lower-${lang}-revealed`, 390);
    if (index === 3) {
      await reload(OPTION);
      check(`${lang} higher-lower reload resumes at next round`, (await pageText()).includes('5/10'));
      const prompt = await page.locator(QUESTION).first().textContent();
      await switchLang(other);
      const switched = await page.locator(QUESTION).first().textContent();
      check(`${lang} higher-lower language switch relocalizes mid game`, prompt !== switched, `${prompt} => ${switched}`);
      await switchLang(lang);
      continue;
    }
    await nextQuestion();
  }
  await page.waitForSelector(RESULT, { state: 'attached', timeout: 10000 });
  const score = await page.locator('[data-testid=quiz-score]').textContent();
  check(`${lang} higher-lower final score`, score === `${expected}/10`, score);
  const streak = await stored(streakKey);
  check(`${lang} higher-lower day streak`, streak?.currentStreak === (wrongAt.size === 0 ? 4 : 0), JSON.stringify(streak));
  await sleep(600);
  await shot(`higher-lower-${lang}-result`, 1440);
  await shot(`higher-lower-${lang}-result`, 390);
  await reload(RESULT);
  check(`${lang} higher-lower result survives reload`, (await page.locator('[data-testid=quiz-score]').textContent()) === score);
  note(`${lang} higher-lower real result ${score}, streak ${streak?.currentStreak}`);
}

function itemPlan(english: any) {
  const shop = itemPoolOn(english.items, today).filter((item: any) => item.cost != null && item.item_tier >= 1 && item.item_tier <= 4);
  const picks = seededShuffle(shop, seededRandom(getModeSeed(today, 'item-stats'))).slice(0, 5);
  const rng = seededRandom(getModeSeed(today, 'item-stats-values'));
  return picks.map((item: any) => ({ item, question: itemStatQuestion(item, rng) }));
}

async function pickItemField(itemId: number, field: string, position: number) {
  await page.locator(`#item-${itemId}-${field} button`).nth(position).dispatchEvent('click');
}

async function playItemStats(lang: Lang, plan: any[], loseTwo: boolean) {
  const other: Lang = lang === 'de' ? 'en' : 'de';
  const key = gameStorageKey('item-stats', today);
  await reset(lang, loseTwo ? undefined : 'item-stats');
  await openMode('item-stats', '[data-testid=item-name]');
  const submitLabel = lang === 'de' ? 'Alle abgeben' : 'Submit all';
  const preMarks = await page.locator('[data-answer-option][data-state=correct], [data-answer-option][data-state=wrong]').count();
  const firstText = await pageContent();
  check(`${lang} item-stats no solution before submit`, preMarks === 0 && !firstText.includes(lang === 'de' ? 'Richtig:' : 'Correct:'));
  const grayscale = await page.locator('main img.grayscale').count();
  check(`${lang} item-stats images grayscale and without alt`, grayscale === plan.length && (await page.locator('main img[alt]:not([alt=""])').count()) === 0, grayscale);
  await page.locator(`button:has-text("${submitLabel}")`).first().dispatchEvent('click');
  await sleep(250);
  check(`${lang} item-stats incomplete submit marks missing fields`, (await pageContent()).includes(lang === 'de' ? 'Bitte wählen' : 'Please choose'));
  const focused = await page.evaluate(() => document.activeElement?.closest('[role=group]')?.id ?? '');
  check(`${lang} item-stats focus moves to first missing field`, focused === `item-${plan[0].item.id}-active`, focused);
  let total = 0;
  let expected = 0;
  for (let index = 0; index < plan.length; index += 1) {
    const { item, question } = plan[index];
    const wrongFields = loseTwo && index === 0;
    total += 3 + (question ? 1 : 0);
    expected += (wrongFields ? 1 : 3) + (question ? 1 : 0);
    await pickItemField(item.id, 'active', item.is_active_item ? 0 : 1);
    await pickItemField(item.id, 'tier', wrongFields ? item.item_tier % 4 : item.item_tier - 1);
    const slotIndex = ['weapon', 'spirit', 'vitality'].indexOf(item.item_slot_type);
    await pickItemField(item.id, 'slot', wrongFields ? (slotIndex + 1) % 3 : slotIndex);
    if (question) await pickItemField(item.id, 'stat', question.options.indexOf(question.value));
    if (index === 0) {
      await page.evaluate(([name, id]: [string, number]) => {
        const state = JSON.parse(localStorage.getItem(name) ?? '{}');
        state.answers[9999] = { tier: 1 };
        state.answers[id].marker = undefined;
        localStorage.setItem(name, JSON.stringify(state));
      }, [key, item.id]);
      await pickItemField(item.id, 'active', item.is_active_item ? 0 : 1);
      await pickItemField(plan[1].item.id, 'tier', plan[1].item.item_tier - 1);
      const mixed = await stored(key);
      check(`${lang} item-stats write keeps value stored behind the tab`, mixed.answers[9999]?.tier === 1 && mixed.answers[plan[1].item.id]?.tier === plan[1].item.item_tier);
    }
    if (index === 1) {
      await shot(`item-stats-${lang}-partial`, 1440);
      await shot(`item-stats-${lang}-partial`, 390);
      await reload('[data-testid=item-name]');
      const kept = await page.locator(`#item-${item.id}-tier button[aria-pressed=true]`).count();
      check(`${lang} item-stats answers survive reload`, kept === 1);
      const names = await page.locator('[data-testid=item-name]').allTextContents();
      const uiBefore = await pageText();
      await switchLang(other);
      const sameNames = await page.locator('[data-testid=item-name]').allTextContents();
      const uiAfter = await pageText();
      check(`${lang} item-stats dashboard switch changes UI, keeps game terms`, sameNames.join() === names.join() && uiBefore.includes(lang === 'de' ? 'Aktivierung' : 'Activation') && uiAfter.includes(lang === 'de' ? 'Activation' : 'Aktivierung'), names[0]);
      await switchLang(lang);
      await switchTerms('german');
      await page.waitForFunction((first: string) => document.querySelector('[data-testid=item-name]')?.textContent !== first, names[0], { timeout: 60000 }).catch(() => undefined);
      const german = await page.locator('[data-testid=item-name]').allTextContents();
      check(`${lang} item-stats terms switch relocalizes names, same items`, german.length === names.length && german.join() !== names.join(), `${names.join()} => ${german.join()}`);
      check(`${lang} item-stats answers kept after terms switch`, (await page.locator(`#item-${item.id}-tier button[aria-pressed=true]`).count()) === 1);
      await switchTerms('english');
    }
  }
  await page.locator(`button:has-text("${submitLabel}")`).first().dispatchEvent('click');
  await page.waitForSelector(RESULT, { state: 'attached', timeout: 10000 });
  const score = await page.locator('[data-testid=quiz-score]').textContent();
  check(`${lang} item-stats final score`, score === `${expected}/${total}`, score);
  const streak = await stored(streakStorageKey('item-stats'));
  check(`${lang} item-stats day streak`, streak?.currentStreak === (loseTwo ? 1 : 4), JSON.stringify(streak));
  await sleep(600);
  await shot(`item-stats-${lang}-result`, 1440);
  await shot(`item-stats-${lang}-result`, 390);
  await reload(RESULT);
  check(`${lang} item-stats result survives reload`, (await page.locator('[data-testid=quiz-score]').textContent()) === score);
  note(`${lang} item-stats real result ${score}, streak ${streak?.currentStreak}`);
}

async function enter(name: string) {
  const input = page.locator('input[role=combobox]');
  await input.fill(name);
  await page.waitForSelector('[role=option]', { state: 'attached', timeout: 5000 });
  const first = await page.locator('[role=option]').first().textContent();
  if (first === name) await input.press('Enter');
  else await page.locator('[role=option]', { hasText: name }).first().dispatchEvent('click');
  await sleep(150);
}
const audioState = () => page.evaluate(() => {
  const audio = document.querySelector('audio');
  return audio ? { src: audio.src.slice(0, 40), paused: audio.paused, time: audio.currentTime, duration: audio.duration, ready: audio.readyState, error: audio.error?.message ?? audio.error?.code ?? null } : null;
});

async function playSound(lang: Lang, shown: any, english: any, tree: unknown, win: boolean) {
  const other: Lang = lang === 'de' ? 'en' : 'de';
  const heroes = heroPoolOn(english.heroes, today);
  const pool = extractSounds(tree, heroes, english.abilities);
  const answerEntry = seededPick(pool.map((entry: any) => ({ ...entry, id: entry.abilityId })), seededRandom(getModeSeed(today, 'guess-sound')));
  const name = (id: number, language = shown) => language.abilities.find((ability: any) => ability.id === id)?.name ?? english.abilities.find((ability: any) => ability.id === id)?.name;
  const answer = name(answerEntry.abilityId);
  const wrongPool = guessableAbilities(english.abilities, heroes).filter((ability: any) => ability.id !== answerEntry.abilityId && name(ability.id).toLowerCase() !== answer.toLowerCase());
  await reset(lang, win ? 'guess-sound' : undefined);
  await openMode('guess-sound', 'input[role=combobox]');
  await page.waitForFunction(() => document.querySelector('audio')?.src.startsWith('blob:'), null, { timeout: 30000 }).catch(() => undefined);
  const text = await pageText();
  check(`${lang} guess-sound answer not visible before end`, !text.includes(answer) && (await attributeLeak([answer])).length === 0, answer);
  const srcBefore = await audioState();
  check(`${lang} guess-sound clip loaded as blob`, srcBefore?.src.startsWith('blob:') === true, JSON.stringify(srcBefore));
  await page.locator('[data-testid=sound-player] button[data-state]').first().dispatchEvent('click');
  await sleep(900);
  const playing = await audioState();
  check(`${lang} guess-sound play accepted without media or CSP error`, playing !== null && !playing.error && playing.paused === false && !(await pageContent()).includes(lang === 'de' ? 'nicht abgespielt' : 'did not play'), JSON.stringify(playing));
  note(`${lang} guess-sound play state ${JSON.stringify(playing)}`);
  await page.locator('[data-testid=sound-player] input[type=range]').evaluate((input: HTMLInputElement) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, '40');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await sleep(150);
  check(`${lang} guess-sound volume stored`, (await stored('deadlockdle:sound-volume')) === 0.4);
  await shot(`guess-sound-${lang}-start`, 1440);
  await shot(`guess-sound-${lang}-start`, 390);
  const firstWrong = wrongPool[0];
  await enter(name(firstWrong.id));
  check(`${lang} guess-sound wrong guess listed`, (await page.locator('[data-testid=previous-guesses] li').allTextContents()).join() === name(firstWrong.id));
  await reload('[data-testid=previous-guesses]');
  check(`${lang} guess-sound guess survives reload`, (await page.locator('[data-testid=previous-guesses] li').count()) === 1);
  check(`${lang} guess-sound volume survives reload`, (await page.locator('[data-testid=sound-player] input[type=range]').inputValue()) === '40');
  const otherName = shownDe.abilities.find((ability: any) => ability.id === firstWrong.id)?.name;
  await switchLang(other);
  await sleep(300);
  check(`${lang} guess-sound dashboard switch keeps game term`, (await page.locator('[data-testid=previous-guesses] li').allTextContents()).join() === name(firstWrong.id));
  await switchLang(lang);
  await switchTerms('german');
  await page.waitForFunction((value: string) => document.querySelector('[data-testid=previous-guesses] li')?.textContent === value, otherName, { timeout: 60000 }).catch(() => undefined);
  const listed = await page.locator('[data-testid=previous-guesses] li').allTextContents();
  check(`${lang} guess-sound guess relocalized by terms switch`, listed.join() === otherName, `${name(firstWrong.id)} => ${listed.join()}`);
  await switchTerms('english');
  if (win) {
    await enter(answer);
  } else {
    for (const ability of wrongPool.slice(1, 4)) await enter(name(ability.id));
  }
  await page.waitForSelector(RESULT, { state: 'attached', timeout: 10000 });
  const shownAnswer = await page.locator('[data-testid=game-answer]').textContent();
  check(`${lang} guess-sound result shows answer`, shownAnswer === answer, `${shownAnswer} / ${answer}`);
  const streak = await stored(streakStorageKey('guess-sound'));
  check(`${lang} guess-sound day streak`, streak?.currentStreak === (win ? 4 : 0), JSON.stringify(streak));
  await sleep(1200);
  await shot(`guess-sound-${lang}-result`, 1440);
  await shot(`guess-sound-${lang}-result`, 390);
  await reload(RESULT);
  check(`${lang} guess-sound result survives reload`, (await page.locator('[data-testid=game-answer]').textContent()) === answer);
  note(`${lang} guess-sound real result ${win ? 'won on guess 2' : 'lost after 4'}, answer ${answer}, streak ${streak?.currentStreak}`);
}

async function hub(english: any) {
  const titles: Record<Lang, string[]> = {
    de: ['Errate den Sound', 'Item-Werte-Quiz', 'Deadlock-Trivia', 'Höher oder niedriger'],
    en: ['Guess the sound', 'Item stats quiz', 'Deadlock trivia', 'Higher or lower'],
  };
  for (const lang of ['de', 'en'] as const) {
    await reset(lang);
    requests.length = 0;
    await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('a[href*="spiel="]', { state: 'attached', timeout: 60000 });
    await sleep(800);
    const tiles = await page.locator('a[href*="spiel="]').count();
    const text = await pageText();
    check(`${lang} hub lists 7 modes with new titles`, tiles === 7 && titles[lang].every((title) => text.includes(title)), tiles);
    check(`${lang} hub requests no sound index`, soundRequests() === 0);
    await shot(`hub-${lang}`, 1440);
    await shot(`hub-${lang}`, 390);
  }
  requests.length = 0;
  for (const [mode, selector] of [['trivia', OPTION], ['item-stats', '[data-testid=item-name]'], ['higher-lower', OPTION], ['guess-hero', 'input[role=combobox]'], ['guess-item', 'input[role=combobox]'], ['guess-ability', 'input[role=combobox]']]) {
    await openMode(mode, selector);
    await sleep(500);
    check(`${mode} reachable via spiel param, no sound index`, soundRequests() === 0);
  }
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('a[href*="spiel=trivia"]', { state: 'attached', timeout: 60000 });
  await page.locator('a[href*="spiel=trivia"]').first().dispatchEvent('click');
  await page.waitForSelector(OPTION, { state: 'attached', timeout: 60000 });
  check('hub link opens trivia in place', page.url().includes('spiel=trivia'));
  for (const mode of ['guess-hero', 'guess-item', 'guess-ability']) {
    await reset('de');
    await openMode(mode, 'input[role=combobox]');
    const pool = mode === 'guess-hero' ? heroPoolOn(english.heroes, today) : mode === 'guess-item' ? itemPoolOn(english.items, today) : [];
    if (pool.length > 0) {
      await enter(pool[0].name);
      check(`${mode} still plays after shared changes`, (await page.locator('[data-testid=previous-guesses] li').count()) >= 1 || (await page.locator(RESULT).count()) === 1);
    }
  }
}

async function currentCspRun(english: any) {
  await reset('de');
  problems.length = 0;
  requests.length = 0;
  await openMode('guess-sound', 'input[role=combobox]');
  await sleep(3000);
  await page.locator('[data-testid=sound-player] button[data-state]').first().dispatchEvent('click').catch(() => undefined);
  await sleep(1500);
  const state = await audioState();
  const text = await pageText();
  note(`current CSP audio ${JSON.stringify(state)}`);
  note(`current CSP error text shown ${text.includes('Der Sound wurde nicht abgespielt')}`);
  note(`current CSP unique errors ${[...new Set(problems)].map((line) => line.slice(0, 200)).join(' || ')}`);
  void english;
  problems.length = 0;
}

async function termsProof(english: any) {
  const pickOf = (mode: string, pool: any[]) => seededPick(pool, seededRandom(getModeSeed(today, mode)));
  const itemPool = itemPoolOn(english.items, today);
  const answer = pickOf('guess-item', itemPool);
  const wrong = itemPool.find((item: any) => item.id !== answer.id && item.name !== 'Extended Magazine');
  const nameDe = (id: number) => shownDe.items.find((item: any) => item.id === id).name;
  const nameEn = (id: number) => english.items.find((item: any) => item.id === id).name;
  const key = gameStorageKey('guess-item', today);
  const COMBO = 'input[role=combobox]';
  const pressed = () => page.locator('[data-game-terms-switch] button[aria-pressed=true]').getAttribute('data-value');

  await reset('de');
  await openMode('guess-item', COMBO);
  const ui = await pageText();
  check('de dashboard default: terms english, UI german', (await pressed()) === 'english' && ui.toLowerCase().includes('zurück zur übersicht') && ui.includes('Spielbegriffe') && ui.includes('Englisch'));
  await page.locator(COMBO).fill('Extended');
  await page.waitForSelector('[role=option]', { state: 'attached', timeout: 5000 });
  const optionsEn = await page.locator('[role=option]').allTextContents();
  check('de dashboard default offers Extended Magazine', optionsEn.includes('Extended Magazine'), optionsEn.join());
  await page.locator(COMBO).fill('');
  await enter(nameEn(wrong.id));
  const listedEn = await page.locator('[data-testid=previous-guesses] li').allTextContents();
  check('de dashboard guess listed with english name', listedEn.join() === nameEn(wrong.id), listedEn.join());
  await shot('terms-de-dashboard-english-terms', 1440);
  await shot('terms-de-dashboard-english-terms', 390);
  const storedBefore = await stored(key);
  check('guesses stored as ids', storedBefore.guesses.join() === String(wrong.id), JSON.stringify(storedBefore.guesses));

  await switchTerms('german');
  await page.waitForFunction((value: string) => document.querySelector('[data-testid=previous-guesses] li')?.textContent === value, nameDe(wrong.id), { timeout: 60000 }).catch(() => undefined);
  const listedDe = await page.locator('[data-testid=previous-guesses] li').allTextContents();
  check('terms deutsch relocalizes retained guess', listedDe.join() === nameDe(wrong.id), `${nameEn(wrong.id)} => ${listedDe.join()}`);
  await page.locator(COMBO).fill('Erweitert');
  await page.waitForSelector('[role=option]', { state: 'attached', timeout: 5000 });
  const optionsDe = await page.locator('[role=option]').allTextContents();
  check('terms deutsch offers Erweitertes Magazin', optionsDe.includes('Erweitertes Magazin'), optionsDe.join());
  await page.locator(COMBO).fill('');
  check('setting key persisted', (await stored('deadlockdle:game-language')) === 'german');
  check('terms switch leaves stored game untouched', JSON.stringify(await stored(key)) === JSON.stringify(storedBefore));
  await reload('[data-testid=previous-guesses]');
  check('reload keeps deutsch and guess', (await pressed()) === 'german' && (await page.locator('[data-testid=previous-guesses] li').allTextContents()).join() === nameDe(wrong.id));
  await enter(nameDe(answer.id));
  await page.waitForSelector(RESULT, { state: 'attached', timeout: 10000 });
  const resultDe = await page.locator('[data-testid=game-answer]').textContent();
  check('same puzzle: german answer name wins', resultDe === nameDe(answer.id), resultDe);
  await switchTerms('english');
  await page.waitForFunction((value: string) => document.querySelector('[data-testid=game-answer]')?.textContent === value, nameEn(answer.id), { timeout: 60000 }).catch(() => undefined);
  const resultEn = await page.locator('[data-testid=game-answer]').textContent();
  check('finished result relocalizes, game stays won', resultEn === nameEn(answer.id) && (await stored(key)).status === 'won', `${resultDe} => ${resultEn}`);

  await reset('en');
  await page.evaluate((name: string) => localStorage.setItem(name, JSON.stringify('german')), 'deadlockdle:game-language');
  const heroes = heroPoolOn(english.heroes, today);
  const abilityPool = buildGuessableAbilities(english.abilities, heroes);
  const abilityAnswer = pickOf('guess-ability', abilityPool);
  const abilityWrong = abilityPool.find((entry: any) => entry.id !== abilityAnswer.id && shownDe.abilities.find((ability: any) => ability.id === entry.id)?.name !== entry.name);
  const abilityDe = shownDe.abilities.find((ability: any) => ability.id === abilityWrong.id).name;
  const legacy = { date: today, guesses: [abilityWrong.id], status: 'playing', hintsRevealed: 1 };
  await page.evaluate(([name, value]: string[]) => localStorage.setItem(name, value), [gameStorageKey('guess-ability', today), JSON.stringify(legacy)]);
  await openMode('guess-ability', '[data-testid=previous-guesses]');
  const enUi = await pageText();
  const abilityListed = await page.locator('[data-testid=previous-guesses] li').allTextContents();
  check('en dashboard with deutsch terms: UI english, names german', enUi.toLowerCase().includes('back to') && enUi.includes('Game terms') && enUi.includes('German') && abilityListed.join() === abilityDe, abilityListed.join());
  check('browser game stored by the live P1 version loads with its guess', abilityListed.length === 1, JSON.stringify(legacy));
  await shot('terms-en-dashboard-german-terms', 1440);
  await shot('terms-en-dashboard-german-terms', 390);

  await reset('de');
  await page.evaluate(() => localStorage.setItem('dashboard.language.v2', 'de'));
  for (const mode of ['guess-hero', 'guess-item', 'guess-sound', 'guess-ability', 'item-stats', 'trivia', 'higher-lower']) {
    await page.goto(`${base}/mini-spiele?spiel=${mode}`, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('[data-game-terms-switch]', { state: 'attached', timeout: 90000 });
    check(`${mode} page has terms switch next to dashboard switch`, (await page.locator('[data-game-terms-switch]').count()) === 1 && (await page.locator('[data-language-switch]').count()) >= 1);
  }
  await page.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('[data-game-terms-switch]', { state: 'attached', timeout: 60000 });
  const second = await context.newPage();
  await second.goto(`${base}/mini-spiele`, { waitUntil: 'domcontentloaded' });
  await second.waitForSelector('[data-game-terms-switch]', { state: 'attached', timeout: 60000 });
  await switchTerms('german');
  const sharedValue = await second.evaluate((name: string) => localStorage.getItem(name), 'deadlockdle:game-language');
  const synced = await second.waitForSelector('[data-game-terms-switch] button[data-value=german][aria-pressed=true]', { state: 'attached', timeout: 4000 }).then(() => true).catch(() => false);
  note(`second tab: storage value ${sharedValue}, native storage event synced ${synced}`);
  if (!synced) {
    await second.evaluate(() => window.dispatchEvent(new StorageEvent('storage')));
    check('terms setting syncs to a second tab through the storage listener', await second.waitForSelector('[data-game-terms-switch] button[data-value=german][aria-pressed=true]', { state: 'attached', timeout: 4000 }).then(() => true).catch(() => false), 'dispatched event, Moli did not deliver a native one');
  } else check('terms setting syncs to a second tab', true);
  await second.close();
  await switchTerms('english');
  await shot('hub-terms-switch', 1440);
}

let shownDe: any = null;
try {
  const english = await languageData('english');
  shownDe = await languageData('german');
  if (runKind === 'current-csp') {
    await currentCspRun(english);
  } else {
    const heroes = heroPoolOn(english.heroes, today);
    const npcs = await fetchApiList('/v1/assets/npc-units', { language: 'english' });
    const questions = generateDailyQuestions(heroes, itemPoolOn(english.items, today), npcs, buildAbilitiesWithHeroes(english.abilities, heroes), seededRandom(getModeSeed(today, 'trivia')));
    const window = statsWindow(today);
    const range = { min_unix_timestamp: String(window.min), max_unix_timestamp: String(window.max), min_average_badge: String(STATS_RANKS.min), max_average_badge: String(STATS_RANKS.max) };
    const abilityHero = new Map<number, number>();
    for (const ability of english.abilities) if (ROUND_TYPES.has(ability.ability_type ?? '') && hasDisplayName(ability) && ability.hero != null) abilityHero.set(ability.id, ability.hero);
    const heroIds = heroes.map((hero: any) => hero.id);
    const abilityHeroes = pickAbilityHeroes(heroIds, today);
    const maxFirst: any[] = [];
    for (const heroId of abilityHeroes) maxFirst[maxFirst.length] = { heroId, rows: await fetchApiList('/v1/analytics/ability-max-first-stats', { ...range, hero_id: String(heroId), min_matches: '1' }) };
    const rounds = buildRounds({ heroIds, abilityHero, maxFirst, heroStats: await fetchApiList('/v1/analytics/hero-stats', range), counters: await fetchApiList('/v1/analytics/hero-counter-stats', { ...range, same_lane_filter: 'false' }), gameStats: (await fetchApiList('/v1/analytics/game-stats', { ...range, bucket: 'no_bucket' }))[0] }, today);
    const tree = ((await fetchApiJson('/v1/assets/sounds', {})) as { abilities?: unknown }).abilities;
    const plan = itemPlan(english);
    note(`trivia questions ${questions.length}, rounds ${rounds.length}, item plan ${plan.map((entry: any) => entry.item.name).join(', ')}`);

    if (sections.has('hub')) await hub(english);
    if (sections.has('terms')) await termsProof(english);
    if (sections.has('trivia')) {
      await playTrivia('de', questions, new Set(), false);
      await playTrivia('en', questions, new Set([1, 4, 8]), true);
    }
    if (sections.has('higher')) {
      await playHigherLower('de', rounds, new Set());
      await playHigherLower('en', rounds, new Set([0, 3, 5, 7, 8, 9, 2]));
    }
    if (sections.has('items')) {
      await playItemStats('de', plan, false);
      await playItemStats('en', plan, true);
    }
    requests.length = 0;
    if (!sections.has('sound')) throw new Error('section end');
    await playSound('de', english, english, tree, true);
    check('guess-sound fetched sound index on open', soundRequests() >= 1, soundRequests());
    await playSound('en', english, english, tree, false);
  }
} catch (error) {
  check('run aborted', false, String(error).slice(0, 500));
  await page.screenshot({ path: `${shotDir}/p2-abort.png`, timeout: 15000 }).catch(() => undefined);
}
const fails = out.filter((line) => line.startsWith('FAIL')).length;
console.log(`checks ok ${out.filter((line) => line.startsWith('ok')).length} failed ${fails}`);
console.log(`page or console problems ${problems.length}, aborted requests ${aborted}`);
console.log([...new Set(problems)].slice(0, 15).join('\n').slice(0, 1800));
await page.close().catch(() => undefined);
await browser.close().catch(() => undefined);
process.exit(0);
