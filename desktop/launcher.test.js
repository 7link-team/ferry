import { beforeEach, afterEach, test, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
let invoke, events;
beforeEach(async () => {
  vi.resetModules();
  document.body.innerHTML = readFileSync('desktop/ui/index.html', 'utf8');
  invoke = vi.fn();
  events = {};
  vi.stubGlobal('__TAURI__', { core: { invoke }, app: { getVersion: async () => '0.1.0-alpha.2' }, event: { listen: async (name, fn) => { events[name] = fn; return () => {}; } } });
  vi.stubGlobal('confirm', vi.fn(() => true));
  await import('./ui/launcher.js');
});
afterEach(() => { vi.unstubAllGlobals(); });
const click = async (id) => { document.getElementById(id).click(); await vi.waitFor(() => expect(document.getElementById('check-update').disabled).toBe(false)); };
test('discovers and installs update without a second confirmation', async () => {
  invoke.mockResolvedValueOnce({ current_version:'0.1.0-alpha.2', version:'0.1.0-alpha.3', notes:'Fixes' });
  await click('check-update');
  expect(document.getElementById('install-update').hidden).toBe(false);
  expect(document.getElementById('update-notes').textContent).toBe('Fixes');
  events['update-progress']({payload:{downloaded:512,total:1024}});
  expect(document.getElementById('update-progress').value).toBe(512);
  invoke.mockResolvedValueOnce(undefined);
  await click('install-update');
  expect(invoke).toHaveBeenLastCalledWith('install_update');
  expect(confirm).not.toHaveBeenCalled();
  expect(document.getElementById('update-status').textContent).toContain('重启');
});
test('latest version and errors leave controls usable', async () => {
  invoke.mockResolvedValueOnce({current_version:'0.1.0-alpha.2'});
  await click('check-update');
  expect(document.getElementById('install-update').hidden).toBe(true);
  expect(document.getElementById('update-status').textContent).toContain('最新版本');
  invoke.mockRejectedValueOnce('network unavailable');
  await click('check-update');
  expect(document.getElementById('update-status').textContent).toBe('network unavailable');
});
test('failed signature is displayed without additional confirmation', async () => {
  invoke.mockResolvedValueOnce({current_version:'0.1.0-alpha.2',version:'0.1.0-alpha.3'});
  await click('check-update');
  invoke.mockRejectedValueOnce('signature invalid');
  await click('install-update');
  expect(document.getElementById('update-status').textContent).toBe('signature invalid');
  expect(document.getElementById('install-update').hidden).toBe(true);
});
test('empty address starts local workspace once and restores controls after failure', async () => {
  let reject;
  invoke.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  document.getElementById('url').value = '   ';
  document.getElementById('open').click();
  expect(invoke).toHaveBeenCalledWith('open_workspace', { serverUrl: '' });
  expect(document.getElementById('open').disabled).toBe(true);
  document.getElementById('open').click();
  expect(invoke).toHaveBeenCalledTimes(1);
  reject('本地服务启动失败');
  await vi.waitFor(() => expect(document.getElementById('open').disabled).toBe(false));
  expect(document.getElementById('connection-error').textContent).toBe('本地服务启动失败');
  invoke.mockResolvedValueOnce(undefined);
  document.getElementById('open').click();
  await vi.waitFor(() => expect(document.getElementById('open').disabled).toBe(false));
  expect(document.getElementById('connection-error').textContent).toBe('');
});

test('upgrade locks controls through drain and installation and exposes failure', async () => {
  let reject;
  invoke.mockResolvedValueOnce({current_version:'0.1.0-alpha.2',version:'0.1.0-alpha.3'});
  await click('check-update');
  invoke.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  document.getElementById('install-update').click();
  for (const id of ['open', 'check-update', 'install-update']) expect(document.getElementById(id).disabled).toBe(true);
  events['update-stage']({ payload: '正在等待本地任务完成并安全关闭服务…' });
  expect(document.getElementById('update-status').textContent).toContain('等待本地任务');
  document.getElementById('install-update').click();
  expect(invoke).toHaveBeenCalledTimes(2);
  reject('本地任务仍在结束中，未安装更新');
  await vi.waitFor(() => expect(document.getElementById('open').disabled).toBe(false));
  expect(document.getElementById('update-status').textContent).toContain('未安装更新');
});
