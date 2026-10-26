import assert from 'node:assert/strict';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const [origin, id, revision] = process.argv.slice(2);
const browser = await chromium.launch({ headless: true });
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const login = await context.request.post(`${origin}/api/sessions`, { headers: { 'X-Ferry-Request': '1' }, data: { email: 'deploy@example.invalid', password: 'correct-horse-battery' } });
  assert.equal(login.status(), 200);
  const page = await context.newPage();
  const errors = []; page.on('pageerror', (e) => errors.push(e.message));
  if (process.argv[5] === 'variables') {
    await page.goto(`${origin}/ops/deploy/apps/${id}`);
    await page.getByRole('button', { name: '预览目标差异', exact: true }).click();
    const bindings = page.getByRole('region', { name: '变量引用' });
    await bindings.waitFor();
    assert((await bindings.textContent()).includes('应用专属'));
    assert((await bindings.textContent()).includes('BOUND_SECRET'));
    assert(!(await page.locator('body').textContent()).includes('fixture-secret-v1'));
    page.on('dialog', dialog => dialog.accept());
    await page.getByRole('button', { name: '执行部署', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="执行输出"]')?.textContent?.includes('部署成功'));
    await page.goto(`${origin}/ops/deploy/revisions/${revision}`);
    await page.getByRole('button', { name: '预览目标差异', exact: true }).click();
    await page.getByText('变量引用 · 历史版本', {exact:true}).waitFor();
    await page.getByText(/已轮换的旧密钥可能失效/).waitFor();
    assert(!(await page.locator('body').textContent()).includes('fixture-secret-v1'));
    assert.deepEqual(errors, []);
    console.log('PASS: variable source/version preview, secret-free execution output, historical rollback warning');
  } else {
  await page.goto(`${origin}/ops/deploy/apps`);
  await page.getByRole('cell', { name: 'runner', exact: true }).waitFor();
  assert.ok(await page.locator('table').count(), 'resource list uses shared table');
  await page.goto(`${origin}/ops/deploy/apps/${id}`);
  await page.getByRole('button', { name: '预览目标差异', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '执行部署', exact: true }).isDisabled(), true);
  await page.getByRole('button', { name: '预览目标差异', exact: true }).click();
  await page.getByRole('heading', { name: '产物差异', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '执行部署', exact: true }).isEnabled(), true);
  page.on('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: '执行部署', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="执行输出"]')?.textContent?.includes('退出码：0'));
  await page.goto(`${origin}/ops/deploy/revisions/${revision}`);
  await page.getByRole('button', { name: /^回滚到 #/ }).waitFor();
  await page.getByRole('button', { name: '预览目标差异', exact: true }).click();
  await page.getByRole('heading', { name: '产物差异', exact: true }).waitFor();
  assert.ok((await page.locator('details').first().locator('pre').textContent()).includes('sha256:'));
  assert.deepEqual(errors, []);
  console.log('PASS: real deployment UI list, preview guard, immutable rollback artifact');
  }
} finally { await browser.close(); }
