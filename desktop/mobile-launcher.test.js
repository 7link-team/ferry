import { afterEach, expect, test, vi } from 'vitest';
import { readFileSync } from 'node:fs';

async function launch(settings = null, hash = '', sharedLocale = '') {
  vi.resetModules();
  history.replaceState({}, '', `/${hash}`);
  document.body.innerHTML = readFileSync('desktop/ui/index.html', 'utf8');
  if (sharedLocale) {
    document.getElementById('connection-controls').dataset.ready = 'true';
    document.getElementById('connection-language').value = sharedLocale;
  }
  vi.stubGlobal('__MOBILE_CLIENT__', true);
  const invoke = vi.fn(async (command) => command === 'connection_settings' ? settings : undefined);
  vi.stubGlobal('__TAURI__', { core: { invoke }, app: { getVersion: async () => '0.1.0' }, event: { listen: async () => () => {} } });
  await import('./ui/launcher.js');
  return invoke;
}
afterEach(() => { vi.unstubAllGlobals(); history.replaceState({}, '', '/'); });

test('mobile uses the shared locale without showing a second language picker', async () => {
  localStorage.setItem('connection-language', 'zh-CN');
  try {
    await launch(null, '#connection-settings', 'ms-MY');
    expect(document.documentElement.lang).toBe('ms-MY');
    expect(document.getElementById('connection-language').hidden).toBe(true);
    expect(document.getElementById('open').textContent).toBe('Buka ruang kerja');
  } finally { localStorage.removeItem('connection-language'); }
});

test('mobile rejects empty address without entering the local server path', async () => {
  const invoke = await launch();
  document.getElementById('open').click();
  await vi.waitFor(() => expect(document.getElementById('open').disabled).toBe(false));
  expect(invoke.mock.calls).toEqual([['connection_settings']]);
  expect(document.getElementById('connection-error').textContent).not.toBe('');
  expect(document.querySelector('.updates').hidden).toBe(false);
  expect(document.getElementById('url').placeholder).toBe('https://');
});

test('mobile fresh process restores saved server but returning to settings does not reconnect', async () => {
  let invoke = await launch({ server_url: 'https://service.example.invalid' });
  expect(invoke.mock.calls).toEqual([['connection_settings'], ['open_workspace', { serverUrl: 'https://service.example.invalid' }]]);
  invoke = await launch({ server_url: 'https://service.example.invalid' }, '#connection-settings');
  expect(invoke.mock.calls).toEqual([['connection_settings']]);
  expect(document.getElementById('url').value).toBe('https://service.example.invalid');
});

test('native history return suppresses auto-restore even when WebView bypasses navigation policy', async () => {
  const invoke = await launch({ server_url: 'https://service.example.invalid', auto_connect: false });
  expect(invoke.mock.calls).toEqual([['connection_settings']]);
  expect(location.hash).toBe('#connection-settings');
  expect(document.getElementById('url').value).toBe('https://service.example.invalid');
});

test('failed mobile connection retains editable URL and permits changing server and retry', async () => {
  const invoke = await launch();
  invoke.mockRejectedValueOnce('server_unavailable');
  document.getElementById('url').value = 'https://offline.example.invalid';
  document.getElementById('open').click();
  await vi.waitFor(() => expect(document.getElementById('connection-error').textContent).not.toBe(''));
  expect(document.getElementById('url').disabled).toBe(false);
  document.getElementById('url').value = 'https://replacement.example.invalid';
  document.getElementById('open').click();
  await vi.waitFor(() => expect(document.getElementById('open').disabled).toBe(false));
  expect(invoke).toHaveBeenLastCalledWith('open_workspace', { serverUrl: 'https://replacement.example.invalid' });
  expect(document.getElementById('connection-error').textContent).toBe('');
});

test('mobile checking locks the submitted address and restores editing on failure', async () => {
  const invoke = await launch();
  let fail;
  invoke.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  document.getElementById('url').value = 'https://offline.example.invalid';
  document.getElementById('open').click();
  expect(document.getElementById('url').disabled).toBe(true);
  fail('server_unavailable');
  await vi.waitFor(() => expect(document.getElementById('url').disabled).toBe(false));
  expect(document.getElementById('url').value).toBe('https://offline.example.invalid');
});

for (const locale of ['zh-CN', 'en-US', 'ms-MY']) {
  test(`mobile launcher renders complete ${locale} connection copy`, async () => {
    vi.stubGlobal('navigator', { language: locale, languages: [locale] });
    await launch();
    expect(document.documentElement.lang).toBe(locale);
    expect(document.getElementById('connection-help').textContent).not.toBe('');
    expect(document.getElementById('url').getAttribute('aria-label')).not.toBeNull();
  });
}
