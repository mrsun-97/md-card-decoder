// End-to-end smoke test of the built site (public/) in headless Chromium, over http:// and file://.
// Usage: node tests/e2e/smoke.mjs [screenshot-dir]
import path from 'node:path';
import { launchBrowser, root } from '../../tools/lib.mjs';
import { createServer } from '../../tools/serve.mjs';

const pub = path.join(root, 'public');
const shots = process.argv[2];
const server = createServer(pub);
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;

const browser = await launchBrowser();
let failures = 0;
const check = (ok, msg) => { console.log(`${ok ? '  ✓' : '  ✗'} ${msg}`); if (!ok) failures++; };

for (const [label, url] of [['http', `http://127.0.0.1:${port}/`], ['file', `file://${pub}/index.html`]]) {
  console.log(label);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector('#dataStats')?.textContent.includes('判定组'), null, { timeout: 30000 });
  check(true, 'boots and loads the card data');
  // Initial reveal: frame = 效果 (2).
  await page.selectOption('#revealField', '0');
  await page.selectOption('#revealValue', '2');
  await page.click('#addRevealBtn');
  await page.waitForFunction(() => !document.querySelector('#proofLabel').hidden, null, { timeout: 30000 });
  const title = await page.textContent('#recommendTitle');
  const count = await page.textContent('#candidateCount');
  check(Number(count.replace(/,/g, '')) > 5000, `initial reveal filters candidates (${count})`);
  check(title && title !== '录入初始揭示', `recommendation shown: ${title} (${await page.textContent('#proofLabel')})`);
  if (shots && label === 'http') await page.screenshot({ path: path.join(shots, 'real-mode.png'), fullPage: true });
  // Test mode: accept the recommended card and let the page judge it, a few times.
  await page.click('#testModeBtn');
  await page.waitForFunction(() => !document.querySelector('#sessionBanner').hidden);
  const opts = await page.$eval('#revealValue', (el) => el.options.length);
  check(opts > 0, `reveal value list is filled after the field list changes (${opts} options)`);
  for (let i = 0; i < 12; i++) {
    const solved = await page.isVisible('#solvedBanner');
    if (solved) break;
    await page.waitForFunction(() => !document.querySelector('#proofLabel').hidden || !document.querySelector('#solvedBanner').hidden, null, { timeout: 30000 });
    if ((await page.textContent('#candidateMass')).startsWith('1 ')) {
      const alts = await page.$$eval('.alt-row', (els) => els.length);
      check(alts === 0, `no wasted alternatives when one candidate is left (${alts} shown)`);
    }
    const isHint = await page.$eval('#recommendTitle', (el) => Boolean(el.dataset.hint));
    await page.click('#recommendTitle');
    if (!isHint) await page.click('#challengeBtn');
    await page.waitForTimeout(50);
  }
  const steps = await page.$$eval('.trail-step', (els) => els.length);
  check(steps >= 2, `活动轨迹 shows ${steps} steps`);
  check(await page.isVisible('#solvedBanner'), 'test-mode puzzle solved by following recommendations');
  if (shots && label === 'http') await page.screenshot({ path: path.join(shots, 'test-mode.png'), fullPage: true });
  // Next puzzle: the new initial reveal must be shown at once and the trail must follow to Q2.
  await page.click('#nextPuzzleBtn');
  const known = await page.$$eval('.field-card.is-known', (els) => els.length);
  check(known === 1, `next puzzle shows its initial reveal (${known} known field)`);
  const tab = await page.textContent('.trail-tab.is-active');
  check(tab.trim().startsWith('Q2'), `活动轨迹 switches to the new puzzle (${tab.trim()})`);
  const nextCount = Number((await page.textContent('#candidateCount')).replace(/,/g, ''));
  check(nextCount > 0 && nextCount < 9059, `candidate count reflects the initial reveal (${nextCount})`);
  check(errors.length === 0, `no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
  await page.close();
}
{
  // Starting mid-event (user's case): puzzle 4, 0 challenges, 3 hints, 4 grant days received;
  // initial reveal DEF 0 and four fields already yellow in the game (game shows 41 candidates).
  console.log('mid-event');
  const ctx = await browser.newContext({ viewport: { width: 1400, height: 1000 } });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForFunction(() => document.querySelector('#dataStats')?.textContent.includes('判定组'), null, { timeout: 30000 });
  const setNum = async (sel, v) => { await page.fill(sel, String(v)); await page.dispatchEvent(sel, 'change'); };
  await setNum('#puzzleInput', 4);
  await setNum('#hintStock', 3);
  await setNum('#challengeStock', 0);
  await setNum('#grantDays', 4);
  const add = async (field, value, source) => {
    await page.selectOption('#revealField', String(field));
    await page.selectOption('#revealValue', String(value));
    await page.selectOption('#revealSource', source);
    await page.click('#addRevealBtn');
  };
  await add(5, 0, 'initial');
  await add(0, 2, 'matched');
  await add(1, 32, 'matched');
  await add(3, 1, 'matched');
  await add(4, 0, 'matched');
  // An earlier challenge with a Fiend card did not match on race: exclude 恶魔族 (8).
  await add(2, 8, 'excluded');
  const groups = await page.textContent('#candidateMass');
  const cards = (await page.textContent('#candidateCount')).trim();
  check(cards === '41', `candidate count matches the game's 41/9058 after excluding 恶魔族 (${groups}, ${cards} cards)`);
  check((await page.inputValue('#puzzleInput')) === '4', 'puzzle number set to 4');
  const planned = await page.textContent('#plannedStock');
  check(planned.includes('提示 8') && planned.includes('挑战 20'), `planned inventory = now + 5 more grant days (${planned})`);
  const matched = await page.textContent('#matchedSummary');
  check(matched.includes('4 / 6'), `补录 fields count as matched, the initial reveal does not (${matched})`);
  await page.waitForFunction(() => !document.querySelector('#proofLabel').hidden, null, { timeout: 30000 });
  const note = await page.textContent('#recommendNote');
  check(note.includes('当前没有挑战次数'), 'zero-challenge note explains the plan is for after the next grant');
  const tabs = await page.$$eval('.trail-tab', (els) => els.map((e) => e.textContent.trim()));
  check(tabs.length === 4 && tabs[0].includes('✓'), `trail shows Q1–Q3 as done and Q4 current (${tabs.join(' ')})`);
  if (shots) await page.screenshot({ path: path.join(shots, 'mid-event.png'), fullPage: true });
  check(errors.length === 0, `no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
  await ctx.close();
}
if (shots) {
  // Phone-width layout.
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2 });
  await phone.goto(`http://127.0.0.1:${port}/`);
  await phone.waitForFunction(() => document.querySelector('#dataStats')?.textContent.includes('判定组'), null, { timeout: 30000 });
  await phone.click('#testModeBtn');
  await phone.waitForFunction(() => !document.querySelector('#proofLabel').hidden, null, { timeout: 30000 });
  const overflow = await phone.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  check(overflow <= 1, `no horizontal page scroll at 390 px (overflow ${overflow}px)`);
  await phone.screenshot({ path: path.join(shots, 'phone.png'), fullPage: true });
  await phone.close();
}
await browser.close();
server.close();
process.exit(failures ? 1 : 0);
