/** Real Chromium layout/interaction acceptance with API fixtures, not backend persistence. */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createServer as createPortServer } from 'node:net';
import { createServer } from 'vite';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');

let server = null;
if(!process.env.CRUD_LAYOUT_UI_URL) {
  // Vite normalizes port 0 to 5173, which may belong to another local app.
  const listener=createPortServer();
  await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));
  const port=listener.address().port;
  await new Promise(resolve=>listener.close(resolve));
  server=await createServer({server:{host:'127.0.0.1',port,strictPort:true,open:false}});
}
if (server) await server.listen();
const origin = process.env.CRUD_LAYOUT_UI_URL || `http://127.0.0.1:${server.httpServer.address().port}`;
const output = resolve('data/maintenance/crud-layout-ui');
await mkdir(output, { recursive: true });
const options = Array.from({ length: 40 }, (_, index) => ({ value: `option-${index + 1}`, label: `Option ${index + 1}` }));
const rows = Array.from({ length: 100 }, (_, index) => ({ id: String(index + 1), key: `layout-${String(index + 1).padStart(3, '0')}-${'long-key-'.repeat(8)}`, name: `Layout dictionary ${index + 1}`, description: 'Layout acceptance fixture', category: 'general', value_type: 'selection', value_json: JSON.stringify(options), items: JSON.stringify(options), schema_json: '{}', value_summary: '40 options', builtin: false, version: 1 }));
const writes = [], errors = [], consoleErrors = [], checks = [], geometry = [];
let canEdit = true, browser, page;
try {
  const chrome=process.env.PLAYWRIGHT_CHROME_PATH||'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ headless: true, ...(existsSync(chrome)? { executablePath: chrome } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1280, height: 800 } });
  await context.route('**/api/**', async route => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    if(!path.startsWith('/api/')) return route.continue();
    const json = value => route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
    if (path === '/api/setup') return json({ required: false, has_users: true, registration_enabled: false });
    if (path === '/api/me') return json({ user: { id: '1', name: 'Layout reviewer', email: 'reviewer@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Layout workspace', slug: 'layout', role_name: 'Reviewer', permissions: ['dictionaries.read', ...(canEdit ? ['dictionaries.create', 'dictionaries.update', 'dictionaries.delete'] : [])] }] });
    if (path.endsWith('/preferences')) return json({ default_page_size: 20, dictionaries: {} });
    const match = /^\/api\/workspaces\/2\/dictionaries(?:\/([^/]+))?$/.exec(path);
    if (match) {
      const id = match[1];
      if (request.method() === 'POST' || request.method() === 'PATCH') {
        assert(canEdit, 'read-only actor must not write');
        const body = request.postDataJSON();
        writes.push({ method: request.method(), id, body });
        const row = { ...body, id: id || String(rows.length + 1), builtin: false, version: Number(body.version || 0) + 1, value_summary: '40 options' };
        if (id) rows.splice(rows.findIndex(old => old.id === id), 1, row); else rows.push(row);
        return json({ id: row.id, version: row.version });
      }
      if (id) return json(rows.find(row => row.id === id));
      const page = Number(url.searchParams.get('page') || 1), size = Number(url.searchParams.get('page_size') || 20);
      return json({ items: rows.slice((page - 1) * size, page * size), total: rows.length, page, page_size: size });
    }
    return json({ items: [], total: 0, page: 1, page_size: 20 });
  });
  page = await context.newPage();
  page.on('pageerror', error => { errors.push(error.message); console.error('Browser error:',error.message); });
  page.on('console',message=>{if(message.type()==='error')consoleErrors.push(message.text());});
  page.setDefaultTimeout(30000);
  page.setDefaultNavigationTimeout(60000);
  const base = `${origin}/zh-CN/workspaces/2/dictionaries`;
  const footer = page.locator('[data-form-actions]');
  const formFrame = async (mode,width) => {
    const result=await page.locator('form').first().evaluate(form=>{
      const rect=element=>{const box=element.getBoundingClientRect();return{x:box.x,y:box.y,width:box.width,height:box.height};};
      const parent=form.parentElement,parentStyle=getComputedStyle(parent),formStyle=getComputedStyle(form),parentBox=parent.getBoundingClientRect();
      const left=parentBox.left+parent.clientLeft+parseFloat(parentStyle.paddingLeft),available=parent.clientWidth-parseFloat(parentStyle.paddingLeft)-parseFloat(parentStyle.paddingRight);
      return{form:rect(form),parent:{left,available},max:parseFloat(formStyle.maxWidth),cards:[...form.querySelectorAll(':scope > section')].map(rect),document:document.documentElement.scrollWidth,viewport:innerWidth};
    });
    assert(Number.isFinite(result.max),`${width}px ${mode}: the shared form has a bounded reading width`);
    assert(Math.abs(result.form.width-Math.min(result.max,result.parent.available))<2,`${width}px ${mode}: shared form fills available width up to its bound: ${JSON.stringify(result)}`);
    assert(Math.abs(result.form.x+result.form.width/2-result.parent.left-result.parent.available/2)<2,`${width}px ${mode}: shared form is centered`);
    assert(result.cards.length>=2,`${width}px ${mode}: fixture exercises multiple schema sections`);
    for(const [index,card] of result.cards.entries()) {
      assert(Math.abs(card.x-result.form.x)<2&&Math.abs(card.width-result.form.width)<2,`${width}px ${mode}: card ${index+1} spans the common form width`);
      if(index)assert(card.y>=result.cards[index-1].y+result.cards[index-1].height-1,`${width}px ${mode}: sections stack without overlap`);
    }
    assert(result.document<=result.viewport+1,`${width}px ${mode}: document must not overflow horizontally`);
    geometry.push({mode,width,...result});
    return result;
  };
  const primaryFillsFooter = async (primary,label) => {
    const result=await footer.evaluate(element=>{
      const style=getComputedStyle(element),buttons=[...element.querySelectorAll(':scope > button,:scope > a')].map(button=>button.getBoundingClientRect().width);
      return{available:element.clientWidth-parseFloat(style.paddingLeft)-parseFloat(style.paddingRight),buttons,gap:parseFloat(style.columnGap)||0};
    });
    assert(Math.abs(result.buttons.reduce((sum,width)=>sum+width,0)+result.gap*(result.buttons.length-1)-result.available)<3,`${label}: actions occupy the footer without an orphan narrow primary button: ${JSON.stringify(result)}`);
    assert((await primary.boundingBox()).width>=Math.max(...result.buttons)-1,`${label}: primary action is the expanding action`);
  };
  const visibleInsideViewport = async (locator, label) => {
    const box = await locator.boundingBox();
    assert(box, `${label} must have a box`);
    const viewport = page.viewportSize();
    assert(box.x >= -1 && box.y >= -1 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1, `${label} must remain wholly visible: ${JSON.stringify({ box, viewport })}`);
    const hit = await locator.evaluate(element => { const box = element.getBoundingClientRect(), hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2); return !!hit && (hit === element || element.contains(hit)); });
    assert(hit, `${label} must not be covered`);
  };
  const scrollForm = async fraction => {
    const scroll = await footer.evaluateHandle(element => { for(let parent = element.parentElement; parent; parent=parent.parentElement) if (/(auto|scroll)/.test(getComputedStyle(parent).overflowY) && parent.scrollHeight > parent.clientHeight) return parent; return null; });
    assert(await scroll.evaluate(element => !!element), 'long form needs a real scroll container');
    await scroll.evaluate((element, part) => { element.scrollTop = (element.scrollHeight - element.clientHeight) * part; }, fraction);
    await page.waitForTimeout(80);
    await scroll.dispose();
  };
  for (const width of [1280, 390, 320]) {
    console.log(`Checking CRUD layout at ${width}px`);
    await page.setViewportSize({ width, height: width === 1280 ? 800 : 844 });
    await page.goto(`${base}/1`);
    await page.getByLabel('选项 40 显示名称', { exact: true }).waitFor();
    const detailFrame=await formFrame('detail',width);
    const edit = footer.getByRole('link', { name: '编辑', exact: true });
    await primaryFillsFooter(edit,`${width}px detail Edit`);
    await scrollForm(0);
    await page.screenshot({path:resolve(output,`detail-top-${width}.png`)});
    for(const fraction of [0, 0.5, 1]) { await scrollForm(fraction); await visibleInsideViewport(edit, `${width}px detail Edit at ${fraction}`); }
    await page.screenshot({ path: resolve(output, `detail-${width}.png`) });
    await edit.click();
    await page.waitForURL(/\/dictionaries\/1\/edit$/);
    const save = footer.getByRole('button', { name: '保存', exact: true });
    await save.waitFor();
    const editFrame=await formFrame('edit',width);
    assert(Math.abs(editFrame.form.width-detailFrame.form.width)<2&&Math.abs(editFrame.form.x-detailFrame.form.x)<2,`${width}px detail and edit share the same frame`);
    await primaryFillsFooter(save,`${width}px edit Save`);
    await scrollForm(0);
    await page.screenshot({path:resolve(output,`edit-top-${width}.png`)});
    for(const fraction of [0, 0.5, 1]) { await scrollForm(fraction); await visibleInsideViewport(save, `${width}px edit Save at ${fraction}`); }
    const last = page.getByLabel('选项 40 显示名称', { exact: true });
    await last.fill(`Last option at ${width}`);
    const fieldBox = await last.boundingBox(), footerBox = await footer.boundingBox();
    assert(fieldBox.y + fieldBox.height <= footerBox.y + 1, `${width}px focused final field must not be hidden under sticky actions`);
    await save.click();
    await page.waitForURL(/\/dictionaries$/);
    assert.equal(JSON.parse(writes.at(-1).body.items)[39].label, `Last option at ${width}`);
    await page.goto(`${base}/new?copy=1`);
    await page.getByLabel('选项 40 显示名称', { exact: true }).waitFor();
    const create = footer.getByRole('button', { name: '创建', exact: true });
    const createFrame=await formFrame('create',width);
    assert(Math.abs(createFrame.form.width-detailFrame.form.width)<2&&Math.abs(createFrame.form.x-detailFrame.form.x)<2,`${width}px detail, edit and create share the same frame`);
    await primaryFillsFooter(create,`${width}px new Create`);
    await scrollForm(0);
    await page.screenshot({path:resolve(output,`create-top-${width}.png`)});
    for(const fraction of [0, 0.5, 1]) { await scrollForm(fraction); await visibleInsideViewport(create, `${width}px new Create at ${fraction}`); }
    await page.getByRole('textbox', { name: /^标识/ }).fill(`created-${width}`);
    await page.getByRole('textbox', { name: /^名称/ }).fill(`Created at ${width}`);
    await create.click();
    await page.waitForURL(/\/dictionaries$/);
    assert.equal(writes.at(-1).body.key, `created-${width}`);
    checks.push(`${width}px centered equal-width detail/edit/create cards, full action footer, long-form sticky actions and saved field`);

    await page.goto(`${base}?page_size=100`);
    const table = page.locator('[data-auto-table-root]');
    await table.getByRole('row').filter({ hasText: 'Layout dictionary 100' }).waitFor();
    const scroller = table.locator(':scope > div[aria-busy]');
    const tableFooter = table.locator('[data-auto-table-footer]');
    const header = table.locator('thead th').first(), rightHeader = table.locator('thead th').last();
    const before = { footer: await tableFooter.boundingBox(), header: await header.boundingBox(), right: await rightHeader.boundingBox() };
    const extent = await scroller.evaluate(element => ({ vertical: element.scrollHeight - element.clientHeight, horizontal: element.scrollWidth - element.clientWidth }));
    assert(extent.vertical > 100 && extent.horizontal > 100, `${width}px fixture must exercise both scroll axes`);
    await scroller.evaluate(element => { element.scrollTop = 300; element.scrollLeft = Math.min(300, element.scrollWidth-element.clientWidth); });
    await page.waitForTimeout(100);
    const after = { footer: await tableFooter.boundingBox(), header: await header.boundingBox(), right: await rightHeader.boundingBox() };
    for(const edge of ['footer','header','right']) { assert(Math.abs(before[edge].y-after[edge].y)<2, `${width}px ${edge} must retain vertical position`); assert(Math.abs(before[edge].x-after[edge].x)<2, `${width}px ${edge} must retain horizontal position`); }
    await visibleInsideViewport(tableFooter.getByRole('navigation'), `${width}px pagination`);
    await visibleInsideViewport(rightHeader, `${width}px action header`);
    await page.screenshot({ path: resolve(output, `table-${width}.png`) });
    checks.push(`${width}px two-axis table header/right actions/pagination`);
  }
  canEdit = false;
  await page.goto(`${base}/1`);
  await page.getByLabel('选项 40 显示名称', { exact: true }).waitFor();
  assert.equal(await page.locator('[data-form-actions]').count(), 0, 'read-only detail must not expose Edit');
  assert.equal(await page.getByLabel('选项 1 显示名称', { exact: true }).isDisabled(), true);
  checks.push('read-only detail hides edit and protects fields');
  assert.deepEqual(errors, []);
  assert.deepEqual(consoleErrors,[]);
  await writeFile(resolve(output,'result.json'),JSON.stringify({scope:'Real Chromium production application with API fixtures; no backend persistence claim',checks,geometry,writes:writes.length,pageErrors:errors,consoleErrors},null,2));
  console.log(`PASS: ${checks.join('; ')}. ${writes.length} fixture writes; no browser errors. API fixtures do not prove backend persistence.`);
} catch(error) {
  if(page) { console.error('Browser errors:',errors);console.error((await page.locator('body').innerText()).slice(0,8000)); await page.screenshot({path:resolve(output,'failure.png'),fullPage:true}); }
  throw error;
} finally {
  if(browser) await browser.close();
  if(server) await server.close();
}
