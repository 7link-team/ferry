const { core, app, event } = window.__TAURI__;
const byId = (id) => document.getElementById(id);
const check = byId("check-update"), install = byId("install-update");
const status = byId("update-status"), progress = byId("update-progress"), notes = byId("update-notes");
core.invoke("release_page").then((url) => {
  const link = byId("release-page"); link.href = url; link.hidden = false;
  link.addEventListener("click", async (event) => {
    event.preventDefault();
    try { await core.invoke("open_release_page"); }
    catch (error) { status.textContent = String(error); }
  });
}).catch((error) => { status.textContent = String(error); });
app.getVersion().then((version) => { byId("version").textContent = `当前版本 ${version}`; })
  .catch(() => { byId("version").textContent = "无法读取当前版本。"; });
async function openWorkspace() {
  byId("connection-error").textContent = "";
  byId("open").disabled = true;
  byId("open").textContent = "正在连接…";
  try { await core.invoke("open_workspace", { serverUrl: byId("url").value.trim() }); }
  catch (error) { byId("connection-error").textContent = String(error); }
  finally { byId("open").disabled = false; byId("open").textContent = "打开工作空间"; }
}
byId("open").addEventListener("click", openWorkspace);
check.addEventListener("click", async () => {
  check.disabled = true; install.hidden = true; notes.hidden = true; progress.hidden = true;
  status.textContent = "正在检查更新…";
  try {
    const result = await core.invoke("check_update");
    byId("version").textContent = `当前版本 ${result.current_version}`;
    if (result.version) {
      status.textContent = `发现版本 ${result.version}。安装时应用将退出并重新启动。`;
      notes.textContent = result.notes || ""; notes.hidden = !result.notes;
      install.hidden = false;
    } else status.textContent = "当前已是本更新通道的最新版本。";
  } catch (error) { status.textContent = String(error); }
  finally { check.disabled = false; }
});
const unlisten = await event.listen("update-progress", ({ payload }) => {
  progress.hidden = false;
  if (payload.total) { progress.max = payload.total; progress.value = payload.downloaded; }
  else progress.removeAttribute("value");
  status.textContent = `已下载 ${(payload.downloaded / 1024 / 1024).toFixed(1)} MB，下载后校验签名。`;
});
window.addEventListener("unload", unlisten, { once: true });
install.addEventListener("click", async () => {
  check.disabled = true; install.disabled = true; byId("open").disabled = true; status.textContent = "正在下载更新…";
  try { await core.invoke("install_update"); status.textContent = "更新已安装，正在重启…"; }
  catch (error) { status.textContent = String(error); install.hidden = true; }
  finally { check.disabled = false; install.disabled = false; byId("open").disabled = false; progress.hidden = true; }
});

const unlistenStage = await event.listen("update-stage", ({ payload }) => { status.textContent = payload; progress.hidden = true; });
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
