import { updateCopy } from './mobile-copy.js';

// This runs only in the bundled launcher. Remote workspace content has no IPC grant.
export async function setupUpdates(getLocale) {
  const byId = (id) => document.getElementById(id);
  const { core, app, event } = window.__TAURI__;
  let current = '';
  let info = null;
  let busy = false;
  let status = 'update_ready';
  const text = () => updateCopy[getLocale()];
  const message = (key) => (text()[key] || text().update_unavailable)
    .replace('{version}', info?.version || '');
  function render() {
    const copy = text();
    byId('update-title').textContent = copy.title;
    byId('version').textContent = copy.current.replace('{version}', current);
    byId('check-update').textContent = copy.check;
    byId('install-update').textContent = info?.platform === 'ios' ? copy.distribution : copy.install;
    byId('check-update').disabled = busy;
    byId('install-update').disabled = busy;
    byId('install-update').hidden = !info?.version || !info.installable;
    const missingSetup = info?.installation_configured === false && status !== 'distribution_missing'
      ? (info.platform === 'android' ? copy.android_signing_missing : copy.ios_distribution_missing) : '';
    byId('update-status').textContent = [message(status), missingSetup].filter(Boolean).join(' ');
    byId('update-notes').textContent = info?.notes || '';
    byId('update-notes').hidden = !info?.notes;
    byId('update-progress').setAttribute('aria-label', copy.progress);
  }
  byId('check-update').addEventListener('click', async () => {
    if (busy) return;
    busy = true; info = null; status = 'checking';
    byId('update-progress').hidden = true; render();
    try {
      info = await core.invoke('mobile_check_update');
      status = !info.version ? 'latest' : !info.installable ? 'distribution_missing'
        : info.platform === 'ios' ? 'available_ios' : 'available';
    } catch (error) { status = String(error); }
    finally { busy = false; render(); }
  });
  byId('install-update').addEventListener('click', async () => {
    if (busy || !info?.installable) return;
    busy = true; status = 'downloading'; render();
    try {
      const result = await core.invoke('mobile_install_update');
      status = result.status;
    } catch (error) { status = String(error); }
    finally { busy = false; byId('update-progress').hidden = true; render(); }
  });
  try {
    await event.listen('mobile-update-progress', ({ payload }) => {
      const progress = byId('update-progress');
      progress.hidden = false;
      if (payload.total > 0) { progress.max = payload.total; progress.value = payload.downloaded; }
      else progress.removeAttribute('value');
    });
    current = await app.getVersion();
  } catch { status = 'update_unavailable'; }
  document.querySelector('.updates').hidden = false;
  render();
  return render;
}
