if (window.top === window) {
window.__MOBILE_CLIENT__ = true;
window.__CLIENT_DOCUMENT_ID__ = Array.from(window.crypto.getRandomValues(new Uint32Array(4)), (part) => part.toString(16).padStart(8, '0')).join('-');
// This unprivileged link is a fallback when the remote app cannot render its own settings.
// Native code only recognizes it as a request to navigate to the bundled document.
const prepareClientShell = () => {
    document.documentElement.dataset.nativeClient = 'mobile';
    window.dispatchEvent(new Event('client-shell:ready'));
    if (!['http:', 'https:'].includes(location.protocol) || location.hostname === 'tauri.localhost') return;
    if (document.getElementById('native-connection-settings')) return;
    const language = document.documentElement.lang || navigator.language || 'en';
    const label = language.startsWith('zh') ? '连接设置' : language.startsWith('ms') ? 'Tetapan sambungan' : 'Connection settings';
    const link = document.createElement('a');
    link.id = 'native-connection-settings';
    link.href = 'app-settings://connection';
    link.textContent = label;
    link.style.cssText = 'position:fixed;z-index:2147483647;right:max(8px,env(safe-area-inset-right));bottom:max(8px,env(safe-area-inset-bottom));max-width:calc(100vw - 16px);min-height:44px;display:flex;align-items:center;padding:8px 12px;background:#161a21;color:#e6e9ee;border:1px solid #98a2b3;border-radius:6px;font:14px/1.4 system-ui;text-decoration:underline;box-sizing:border-box';
    document.body.append(link);
    const updateVisibility = () => {
      link.hidden = !!document.querySelector('[data-client-connection-entry]');
      link.style.display = link.hidden ? 'none' : 'flex';
    };
    updateVisibility();
    const observer = new MutationObserver(updateVisibility);
    const observe = () => observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-client-connection-entry'] });
    observe();
    window.addEventListener('pagehide', () => observer.disconnect(), { once: true });
    window.addEventListener('pageshow', () => { observe(); updateVisibility(); });
};
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', prepareClientShell, { once: true });
else prepareClientShell();
}
