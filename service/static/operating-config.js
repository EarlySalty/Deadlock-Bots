/* Bestehende Dashboard-Anmeldung und fetchJSON verwenden. Keine Diensttokens. */
(() => {
  'use strict';
  let loading = false;
  let mounted = false;
  window.loadOperatingConfig = async () => {
    if (loading || mounted) return;
    const root = document.getElementById('operating-config-cards');
    if (!root) return;
    loading = true;
    root.textContent = 'Bot-Konfigurationen werden geladen …';
    try {
      if (!document.getElementById('bot-config-editor-style')) {
        const style = document.createElement('link'); style.id = 'bot-config-editor-style';
        style.rel = 'stylesheet'; style.href = '/api/admin/bot-config-editor/css';
        document.head.append(style);
      }
      if (!window.DdcBotConfigEditor) {
        await new Promise((resolve, reject) => {
          const script = document.createElement('script'); script.src = '/api/admin/bot-config-editor/js';
          script.onload = resolve;
          script.onerror = () => { script.remove(); reject(new Error('Die Konfigurationsoberfläche konnte nicht geladen werden. Bitte die Seite neu laden.')); };
          document.head.append(script);
        });
      }
      if (!window.DdcBotConfigEditor) throw new Error('Die Konfigurationsoberfläche ist nicht verfügbar.');
      window.DdcBotConfigEditor.mount(root, {
        request: (url, options) => fetchJSON(url, options),
        listEndpoint: '/api/admin/bot-configs',
        endpoint: id => `/api/admin/bot-configs/${encodeURIComponent(id)}`,
      });
      mounted = true;
    } catch (error) {
      root.textContent = error instanceof Error ? error.message : 'Die Konfigurationsoberfläche konnte nicht geladen werden.';
    } finally { loading = false; }
  };
  if (location.hash === '#betrieb') void window.loadOperatingConfig();
})();
