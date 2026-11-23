import { afterEach, expect, test, vi } from 'vitest';
import { readFileSync } from 'node:fs';

async function launch(result = {}) {
  vi.resetModules();
  history.replaceState({}, '', '/#connection-settings');
  document.body.innerHTML = readFileSync('desktop/ui/index.html', 'utf8');
  const handlers = {};
  const invoke = vi.fn(async (command) => {
    if (command === 'connection_settings') return null;
    if (command === 'mobile_check_update') return result;
    return { status: 'installer_opened' };
  });
  vi.stubGlobal('__MOBILE_CLIENT__', true);
  vi.stubGlobal('__TAURI__', { core: { invoke }, app: { getVersion: async () => '1.0.0' },
    event: { listen: async (name, handler) => { handlers[name] = handler; return () => {}; } } });
  await import('./ui/launcher.js');
  return { invoke, handlers };
}
afterEach(() => { vi.unstubAllGlobals(); localStorage.clear(); });

test('mobile exposes version check and renders untrusted release notes as text', async () => {
  const { invoke } = await launch({ version: '1.1.0', installable: true,
    notes: '<img src=x onerror=alert(1)>', platform: 'android' });
  expect(document.querySelector('.updates').hidden).toBe(false);
  expect(document.getElementById('version').textContent).toContain('1.0.0');
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('install-update').hidden).toBe(false));
  expect(invoke).toHaveBeenCalledWith('mobile_check_update');
  expect(document.getElementById('update-notes').textContent).toContain('<img');
  expect(document.getElementById('update-notes').querySelector('img')).toBeNull();
});

test('iOS with no distribution URL reports available version without install action', async () => {
  await launch({ version: '1.1.0', installable: false, platform: 'ios' });
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('update-status').textContent).toContain('1.1.0'));
  expect(document.getElementById('install-update').hidden).toBe(true);
});

test('configured iOS update opens its distribution channel and retains version on language change', async () => {
  const { invoke } = await launch({ version: '1.1.0', installable: true, platform: 'ios' });
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('install-update').hidden).toBe(false));
  const language = document.getElementById('connection-language');
  language.value = 'en-US'; language.dispatchEvent(new Event('change'));
  expect(document.getElementById('update-status').textContent).toContain('iOS');
  expect(document.getElementById('update-status').textContent).not.toContain('Android');
  invoke.mockResolvedValueOnce({ status: 'distribution_opened' });
  document.getElementById('install-update').click();
  await vi.waitFor(() => expect(document.getElementById('update-status').textContent).toContain('distribution page'));
  expect(invoke).toHaveBeenLastCalledWith('mobile_install_update');
});

test('check failures permit retry and never leave a stale install action', async () => {
  const { invoke } = await launch({ version: '1.1.0', installable: true, platform: 'android' });
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('install-update').hidden).toBe(false));
  invoke.mockRejectedValueOnce('update_unavailable');
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('check-update').disabled).toBe(false));
  expect(document.getElementById('install-update').hidden).toBe(true);
  expect(document.getElementById('update-status').textContent).not.toContain('update_unavailable');
});

test('missing Android signing remains explicit even when no newer version exists', async () => {
  localStorage.setItem('connection-language', 'en-US');
  await launch({ version: null, installation_configured: false, platform: 'android' });
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('update-status').textContent).toContain('No persistent Android signing key'));
  expect(document.getElementById('install-update').hidden).toBe(true);
});

test('system install permission remains retryable and download progress is visible', async () => {
  const { invoke, handlers } = await launch({ version: '1.1.0', installable: true, platform: 'android' });
  document.getElementById('check-update').click();
  await vi.waitFor(() => expect(document.getElementById('install-update').hidden).toBe(false));
  invoke.mockResolvedValueOnce({ status: 'permission_required' });
  document.getElementById('install-update').click();
  handlers['mobile-update-progress']({ payload: { downloaded: 20, total: 100 } });
  expect(document.getElementById('update-progress').value).toBe(20);
  await vi.waitFor(() => expect(document.getElementById('install-update').disabled).toBe(false));
  expect(document.getElementById('install-update').hidden).toBe(false);
});

for (const locale of ['zh-CN', 'en-US', 'ms-MY']) {
  test(`mobile update controls use ${locale} copy`, async () => {
    localStorage.setItem('connection-language', locale);
    await launch({ version: null, platform: 'android' });
    document.getElementById('check-update').click();
    await vi.waitFor(() => expect(document.getElementById('check-update').disabled).toBe(false));
    for (const id of ['update-title', 'version', 'check-update', 'update-status']) {
      expect(document.getElementById(id).textContent.trim()).not.toBe('');
    }
    expect(document.documentElement.lang).toBe(locale);
  });
}
