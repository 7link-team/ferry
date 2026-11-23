import { copy } from './mobile-copy.js';
import { setupUpdates } from './mobile-update.js';

const byId = (id) => document.getElementById(id);
const { core } = window.__TAURI__;
document.documentElement.dataset.nativeClient = 'mobile';
const preferred = navigator.languages?.[0] || navigator.language || 'en-US';
let locale = preferred.startsWith('zh') ? 'zh-CN' : preferred.startsWith('ms') ? 'ms-MY' : 'en-US';
try { locale = localStorage.getItem('connection-language') || locale; } catch { /* OS storage may be unavailable. */ }
if (byId('connection-controls')?.dataset.ready) locale = byId('connection-language').value;
let busy = false;
let renderUpdates = () => {};
let errorKey = location.hash.includes('navigation-failed') ? 'navigation_failed' : '';
function render() {
  const text = copy[locale] || copy['en-US'];
  document.documentElement.lang = locale;
  document.title = text.title;
  for (const [id, key] of [['connection-title', 'title'], ['connection-help', 'help'], ['url-label', 'address']]) byId(id).textContent = text[key];
  byId('url').setAttribute('aria-label', text.address);
  byId('connection-language').setAttribute('aria-label', text.language);
  byId('open').textContent = busy ? text.connecting : text.open;
  byId('connection-error').textContent = text[errorKey] || errorKey;
  renderUpdates();
}
byId('url').placeholder = 'https://';
byId('connection-language').hidden = !!byId('connection-controls')?.dataset.ready;
byId('connection-language').value = locale;
byId('connection-language').addEventListener('change', () => {
  locale = byId('connection-language').value;
  try { localStorage.setItem('connection-language', locale); } catch { /* Keep current selection. */ }
  render();
});
async function openWorkspace() {
  if (busy) return;
  errorKey = '';
  const serverUrl = byId('url').value.trim();
  try {
    const url = new URL(serverUrl);
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error();
  } catch {
    errorKey = 'invalid_url'; render(); byId('url').focus(); return;
  }
  busy = true; byId('open').disabled = true; byId('url').disabled = true; render();
  try { await core.invoke('open_workspace', { serverUrl }); }
  catch (error) { errorKey = String(error); }
  finally { busy = false; byId('open').disabled = false; byId('url').disabled = false; render(); }
}
byId('open').addEventListener('click', openWorkspace);
byId('url').addEventListener('keydown', (event) => { if (event.key === 'Enter') void openWorkspace(); });
render();
try {
  const settings = await core.invoke('connection_settings');
  if (settings) {
    byId('url').value = settings.server_url;
    if (settings.auto_connect === false && !location.hash.startsWith('#connection-settings')) {
      history.replaceState(null, '', '#connection-settings');
    }
    if (!location.hash.startsWith('#connection-settings')) await openWorkspace();
  }
} catch (error) { errorKey = String(error); render(); }
renderUpdates = await setupUpdates(() => locale);
