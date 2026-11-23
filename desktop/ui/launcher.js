import { copy, updateCopy, desktopCopy } from './mobile-copy.js';
if (window.__MOBILE_CLIENT__) {
  await import('./mobile-launcher.js');
} else {
document.querySelector('.updates').hidden = false;
const { core, app, event } = window.__TAURI__;
const byId = (id) => document.getElementById(id);
const check = byId("check-update"), install = byId("install-update");
const status = byId("update-status"), progress = byId("update-progress"), notes = byId("update-notes");
let locale = byId('connection-controls')?.dataset.ready ? byId('connection-language').value : 'zh-CN';
let version = '', statusKey = '', statusValues = {}, rawStatus = '', opening = false;
const text = () => ({ ...copy[locale] || copy['en-US'], ...updateCopy[locale] || updateCopy['en-US'], ...desktopCopy[locale] || desktopCopy['en-US'] });
const format = (key, values = {}) => Object.entries(values).reduce((result, [name, value]) => result.replace(`{${name}}`, String(value)), text()[key] || key);
function render() {
  document.documentElement.lang = locale;
  document.title = text().title;
  for (const [id, key] of [['connection-title','title'], ['connection-help','help'], ['url-label','address'], ['update-title','title'], ['check-update','check'], ['install-update','install'], ['release-page','release']]) byId(id).textContent = text()[key];
  byId('update-title').textContent = (updateCopy[locale] || updateCopy['en-US']).title;
  byId('open').textContent = text()[opening ? 'connecting' : 'open'];
  byId('version').textContent = version ? format('current', {version}) : text().version_error;
  byId('url').setAttribute('aria-label', text().address);
  progress.setAttribute('aria-label', text().progress);
  status.textContent = rawStatus || (statusKey ? format(statusKey, statusValues) : '');
}
function setStatus(key, values = {}, raw = '') { statusKey = key; statusValues = values; rawStatus = raw; render(); }
byId('connection-language').addEventListener('change', () => { locale = byId('connection-language').value; render(); });
render();
core.invoke("release_page").then((url) => {
  const link = byId("release-page"); link.href = url; link.hidden = false;
  link.addEventListener("click", async (event) => {
    event.preventDefault();
    try { await core.invoke("open_release_page"); }
    catch (error) { setStatus('', {}, String(error)); }
  });
}).catch((error) => { setStatus('', {}, String(error)); });
app.getVersion().then((value) => { version = value; render(); }).catch(() => { version = ""; render(); });
async function openWorkspace() {
  byId("connection-error").textContent = "";
  byId("open").disabled = true;
  opening = true; render();
  try { await core.invoke("open_workspace", { serverUrl: byId("url").value.trim() }); }
  catch (error) { byId("connection-error").textContent = String(error); }
  finally { byId("open").disabled = false; opening = false; render(); }
}
byId("open").addEventListener("click", openWorkspace);
byId("url").addEventListener("keydown", event => { if (event.key === "Enter" && !byId("open").disabled) void openWorkspace(); });
check.addEventListener("click", async () => {
  check.disabled = true; install.hidden = true; notes.hidden = true; progress.hidden = true;
  setStatus("checking");
  try {
    const result = await core.invoke("check_update");
    version = result.current_version; render();
    if (result.version) {
      setStatus("available", { version: result.version });
      notes.textContent = result.notes || ""; notes.hidden = !result.notes;
      install.hidden = false;
    } else setStatus("latest");
  } catch (error) { setStatus('', {}, String(error)); }
  finally { check.disabled = false; }
});
const unlisten = await event.listen("update-progress", ({ payload }) => {
  progress.hidden = false;
  if (payload.total) { progress.max = payload.total; progress.value = payload.downloaded; }
  else progress.removeAttribute("value");
  setStatus("downloaded", { size: (payload.downloaded / 1024 / 1024).toFixed(1) });
});
window.addEventListener("unload", unlisten, { once: true });
install.addEventListener("click", async () => {
  check.disabled = true; install.disabled = true; byId("open").disabled = true; setStatus("downloading");
  try { await core.invoke("install_update"); setStatus("installed"); }
  catch (error) { setStatus('', {}, String(error)); install.hidden = true; }
  finally { check.disabled = false; install.disabled = false; byId("open").disabled = false; progress.hidden = true; }
});

const unlistenStage = await event.listen("update-stage", ({ payload }) => { setStatus("", {}, payload); progress.hidden = true; });
window.addEventListener("unload", unlistenStage, { once: true });

const unlistenConnection = await event.listen("connection-error", ({ payload }) => { byId("connection-error").textContent = payload; });
window.addEventListener("unload", unlistenConnection, { once: true });

// Restore after listeners are ready so startup failures stay visible.
try {
  const settings = await core.invoke("connection_settings");
  if (settings) {
    byId("url").value = settings.server_url;
    await openWorkspace();
  }
} catch (error) {
  byId("connection-error").textContent = String(error);
}
}
