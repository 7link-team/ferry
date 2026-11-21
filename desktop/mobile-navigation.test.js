import { afterEach, expect, test, vi } from 'vitest';
import { readFileSync } from 'node:fs';

afterEach(() => { document.body.innerHTML = ''; vi.unstubAllGlobals(); });
test('native shell announces itself and fallback yields to the shared visible connection entry', async () => {
  document.body.innerHTML = '<main>Workspace</main>';
  const ready = vi.fn();
  window.addEventListener('client-shell:ready', ready, { once: true });
  const nativeNavigation = { href: '', protocol: 'http:', hostname: 'server.example.invalid' };
  new Function('window', 'location', readFileSync('desktop/ui/mobile-navigation.js', 'utf8'))(window, nativeNavigation);
  document.dispatchEvent(new Event('DOMContentLoaded'));
  expect(document.documentElement.dataset.nativeClient).toBe('mobile');
  expect(ready).toHaveBeenCalledOnce();
  expect(window.__CLIENT_DOCUMENT_ID__).toMatch(/^[a-f0-9]{8}(?:-[a-f0-9]{8}){3}$/);
  expect(nativeNavigation.href).toBe('');
  const fallback = document.getElementById('native-connection-settings');
  expect(fallback.href).toBe('app-settings://connection');
  expect(fallback.hidden).toBe(false);
  const entry = document.createElement('button');
  entry.dataset.clientConnectionEntry = '';
  document.body.append(entry);
  await vi.waitFor(() => expect(fallback.hidden).toBe(true));
  expect(getComputedStyle(fallback).display).toBe('none');
  entry.remove();
  await vi.waitFor(() => expect(fallback.hidden).toBe(false));
  expect(getComputedStyle(fallback).display).not.toBe('none');
});

test('child-frame initialization cannot replace top-document native markers or nonce', () => {
  const frame = { top: {}, crypto: window.crypto };
  const nativeNavigation = { href: '' };
  new Function('window', 'location', readFileSync('desktop/ui/mobile-navigation.js', 'utf8'))(frame, nativeNavigation);
  expect(frame.__MOBILE_CLIENT__).toBeUndefined();
  expect(frame.__CLIENT_DOCUMENT_ID__).toBeUndefined();
  expect(nativeNavigation.href).toBe('');
});
