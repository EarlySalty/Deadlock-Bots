/* Gemeinsame Admin-Oberfläche. Keine Diensttokens, kein HTML aus Konfigurationen. */
(() => {
  'use strict';
  const make = (tag, text, className) => {
    const node = document.createElement(tag);
    if (text !== undefined) node.textContent = text;
    if (className) node.className = className;
    return node;
  };
  const messageOf = error => {
    if (!(error instanceof Error)) return 'Die Anfrage konnte nicht abgeschlossen werden.';
    try {
      const value = JSON.parse(error.message);
      if (typeof value.message === 'string') return value.message;
      if (typeof value.error === 'string') return value.error;
    } catch (_) { /* Bestehende Dashboard-Helfer liefern auch einfache Fehlermeldungen. */ }
    return error.message;
  };
  window.DdcBotConfigEditor = {
    mount(root, options) {
      root.replaceChildren(); root.classList.add('bot-config-editor');
      const request = options.request;
      const state = { bot: null, saved: null, baseline: '', busy: false, disposed: false, epoch: 0, timer: null, polls: 0 };
      const heading = make('div', undefined, 'bce-heading');
      const intro = make('div');
      intro.append(make('h2', 'Bot-Konfiguration'), make('p', 'TOML bearbeiten, prüfen und direkt übernehmen. Zugangsdaten bleiben in Infisical.'));
      const selector = make('select'); selector.setAttribute('aria-label', 'Bot auswählen');
      heading.append(intro, selector);
      const error = make('p', '', 'bce-error'); error.setAttribute('role', 'alert');
      const note = make('p', '', 'bce-note');
      const status = make('div', '', 'bce-status'); status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
      const serviceList = make('div', '', 'bce-services');
      const editorLabel = make('label', 'Nicht geheime Betriebseinstellungen (TOML)', 'bce-label');
      const editor = make('textarea'); editor.spellcheck = false; editor.wrap = 'off';
      editor.setAttribute('aria-label', 'TOML-Konfiguration'); editor.setAttribute('autocomplete', 'off');
      editor.setAttribute('autocapitalize', 'off'); editor.setAttribute('autocorrect', 'off'); editor.rows = 25;
      const protectedText = make('p', '', 'bce-note');
      const preview = make('div', '', 'bce-preview'); preview.setAttribute('role', 'status');
      const buttons = make('div', '', 'bce-actions');
      const validate = make('button', 'Änderungen prüfen');
      const save = make('button', 'Speichern', 'bce-primary');
      const apply = make('button', 'Übernehmen & neu starten', 'bce-primary');
      const reload = make('button', 'Neu laden');
      for (const button of [validate, save, apply, reload]) { button.type = 'button'; buttons.append(button); }
      const footer = make('div', '', 'bce-footer');
      const history = make('select'); history.setAttribute('aria-label', 'Gesicherte Version auswählen');
      const restore = make('button', 'Version als Entwurf laden'); restore.type = 'button';
      const revision = make('span', '', 'bce-revision');
      footer.append(history, restore, revision);
      root.append(heading, error, note, status, serviceList, editorLabel, editor, protectedText, preview, buttons, footer,
        make('p', 'Speichern ändert die Datei. Erst „Übernehmen & neu starten“ startet die zugehörigen Dienste. Dabei sind sie kurz nicht erreichbar. Frühere Versionen werden als Entwurf geladen und nicht automatisch aktiviert.', 'bce-note'));
      function dirty() { return state.saved !== null && editor.value !== state.baseline; }
      function enabled() {
        if (typeof options.onDirtyChange === 'function') options.onDirtyChange(dirty());
        const ready = Boolean(state.saved && state.bot && state.bot.available);
        editor.disabled = state.busy || !ready;
        selector.disabled = state.busy;
        validate.disabled = state.busy || !ready;
        save.disabled = state.busy || !ready || !dirty();
        apply.disabled = state.busy || !ready || dirty();
        reload.disabled = state.busy || !state.bot || !state.bot.available;
        history.disabled = state.busy || !ready;
        restore.disabled = state.busy || !ready || !history.value;
        reload.textContent = dirty() ? 'Entwurf verwerfen & neu laden' : 'Neu laden';
        revision.textContent = state.saved ? `Stand ${state.saved.revision.slice(0, 12)}${dirty() ? ' · Ungespeicherter Entwurf' : ''}` : '';
      }
      function stopPolling() { if (state.timer !== null) window.clearTimeout(state.timer); state.timer = null; }
      function renderRuntime(runtime) {
        serviceList.replaceChildren();
        if (!runtime) { status.textContent = 'Gespeichert. Der aktive Stand der Dienste ist noch nicht bestätigt.'; return; }
        const op = runtime.operation;
        if (op && (op.status === 'queued' || op.status === 'running')) {
          const recent = Date.now() / 1000 - op.at < 180;
          status.textContent = recent ? op.message : 'Die Aktivierung wurde noch nicht bestätigt. Bitte den Dienststatus prüfen.';
        } else if (runtime.saved_revision_active) {
          status.textContent = 'Gespeicherter Stand nach Neustart aktiv.';
        } else if (op && op.status === 'failed') {
          status.textContent = op.message;
        } else {
          status.textContent = 'Gespeicherter Stand vorhanden. Übernahme durch die laufenden Dienste noch nicht bestätigt.';
        }
        for (const service of runtime.services || []) {
          const label = service.state === 'active'
            ? (service.config_connected ? 'läuft mit dieser TOML' : 'noch nicht an diese TOML angeschlossen')
            : (service.state === 'failed' ? 'Dienst fehlgeschlagen' : 'nicht aktiv');
          serviceList.append(make('span', `${service.unit.replace(/\.service$/, '')}: ${label}`, 'bce-service'));
        }
      }
      function render(payload) {
        const saved = payload.snapshot;
        state.saved = saved; state.baseline = saved.toml; editor.value = saved.toml;
        protectedText.textContent = saved.protected.length
          ? `Geschützte Felder (bleiben unverändert): ${saved.protected.join(', ')}`
          : 'Die Schema-Version und Zugangsdaten sind nicht im Dashboard änderbar.';
        history.replaceChildren(make('option', 'Gesicherte Version auswählen'));
        history.firstElementChild.value = '';
        for (const hash of saved.history) {
          if (hash === saved.revision) continue;
          const option = make('option', hash.slice(0, 16)); option.value = hash; history.append(option);
        }
        renderRuntime(payload.runtime); preview.replaceChildren(); enabled();
      }
      const endpoint = () => options.endpoint(state.bot.id);
      const post = data => request(endpoint(), { method: 'POST', body: JSON.stringify(data) });
      async function guarded(operation) {
        if (state.busy || state.disposed) return;
        state.busy = true; error.textContent = ''; enabled();
        try { await operation(); }
        catch (problem) { if (!state.disposed) error.textContent = messageOf(problem); }
        finally { state.busy = false; if (!state.disposed) enabled(); }
      }
      async function load() {
        if (!state.bot || !state.bot.available) return;
        const epoch = ++state.epoch;
        const payload = await request(endpoint(), { cache: 'no-store' });
        if (!state.disposed && epoch === state.epoch) render(payload);
      }
      function allowDiscard() { return !dirty() || window.confirm('Den ungespeicherten Entwurf verwerfen?'); }
      function showChanges(result) {
        preview.replaceChildren();
        preview.append(make('strong', result.changed.length ? `${result.changed.length} geänderte Einstellungen – Prüfung bestanden` : 'Keine inhaltlichen Änderungen.'));
        if (result.changed.length) preview.append(make('p', result.changed.join(' · ')));
      }
      async function poll() {
        if (state.disposed || !state.saved || state.polls >= 60) {
          if (!state.disposed && state.polls >= 60) status.textContent = 'Aktivierung noch nicht bestätigt. Mit „Neu laden“ erneut prüfen.';
          return;
        }
        state.polls += 1;
        const epoch = state.epoch;
        try {
          const data = await post({ action: 'status', revision: state.saved.revision });
          if (state.disposed || epoch !== state.epoch) return;
          if (data.revision !== state.saved.revision) {
            status.textContent = 'Die Datei wurde inzwischen erneut geändert. Bitte neu laden; dein Entwurf bleibt bis dahin erhalten.';
            return;
          }
          renderRuntime(data.runtime);
          const op = data.runtime && data.runtime.operation;
          if (op && (op.status === 'applied' || op.status === 'failed')) return;
        } catch (_) {
          // Neustart des Webdienstes: nicht jede erfolglose Statusprobe als Fehler melden.
          if (!state.disposed && epoch === state.epoch) status.textContent = 'Neustart läuft. Die Verbindung zum Dashboard wird wiederhergestellt …';
        }
        if (!state.disposed && epoch === state.epoch) state.timer = window.setTimeout(poll, 3000);
      }
      editor.addEventListener('input', () => { preview.replaceChildren(); enabled(); });
      history.addEventListener('change', enabled);
      validate.addEventListener('click', () => guarded(async () => {
        const result = await post({ action: 'validate', revision: state.saved.revision, toml: editor.value });
        if (!state.disposed) showChanges(result);
      }));
      save.addEventListener('click', () => guarded(async () => {
        const text = editor.value;
        const result = await post({ action: 'validate', revision: state.saved.revision, toml: text });
        if (state.disposed) return;
        showChanges(result);
        if (!result.changed.length) return;
        if (!window.confirm(`${result.changed.length} Einstellungen für ${state.bot.title} speichern?\n\n${result.changed.slice(0, 20).join('\n')}\n\nDie Dienste werden dabei noch nicht neu gestartet.`)) return;
        const saved = await post({ action: 'save', revision: state.saved.revision, toml: text });
        if (!state.disposed) render(saved);
      }));
      apply.addEventListener('click', () => guarded(async () => {
        if (dirty() || !window.confirm(`${state.bot.title}: Gespeicherte Konfiguration übernehmen und die zugehörigen Dienste neu starten? Dabei sind sie kurz nicht erreichbar.`)) return;
        await post({ action: 'activate', revision: state.saved.revision });
        if (state.disposed) return;
        status.textContent = 'Aktivierung angefordert. Neustart und neue Prozesse werden geprüft …';
        stopPolling(); state.polls = 0; state.timer = window.setTimeout(poll, 3000);
      }));
      reload.addEventListener('click', () => { if (allowDiscard()) { stopPolling(); void guarded(load); } });
      restore.addEventListener('click', () => guarded(async () => {
        if (!allowDiscard()) return;
        const data = await post({ action: 'history', revision: history.value });
        if (state.disposed) return;
        editor.value = data.toml; preview.replaceChildren(make('p', 'Frühere Version als Entwurf geladen. Bitte prüfen und anschließend speichern.'));
      }));
      selector.addEventListener('change', () => {
        if (!allowDiscard()) { selector.value = state.bot ? state.bot.id : ''; return; }
        stopPolling(); state.epoch += 1;
        state.bot = bots.find(bot => bot.id === selector.value) || null;
        state.saved = null; state.baseline = ''; editor.value = ''; preview.replaceChildren(); serviceList.replaceChildren();
        history.replaceChildren(); error.textContent = ''; note.textContent = state.bot ? state.bot.note : '';
        status.textContent = state.bot && state.bot.available ? 'Konfiguration wird geladen …' : 'Für diesen Bot ist noch keine aktive, validierbare TOML-Verbindung eingerichtet.';
        enabled(); if (state.bot && state.bot.available) void guarded(load);
      });
      function beforeUnload(event) { if (dirty()) { event.preventDefault(); event.returnValue = ''; } }
      window.addEventListener('beforeunload', beforeUnload);
      let bots = [];
      enabled(); status.textContent = 'Verfügbare Bots werden geprüft …';
      void guarded(async () => {
        bots = options.single ? [{ id: 'twitch', title: 'Twitch-Bot', available: true, note: 'Twitch wird ausschließlich in diesem Admin-Dashboard verwaltet.' }]
          : (await request(options.listEndpoint, { cache: 'no-store' })).bots;
        if (state.disposed) return;
        for (const bot of bots) {
          const option = make('option', `${bot.title}${bot.available ? '' : ' – noch nicht verbunden'}`); option.value = bot.id; selector.append(option);
        }
        selector.hidden = Boolean(options.single);
        state.bot = bots.find(bot => bot.available) || bots[0] || null;
        if (!state.bot) { status.textContent = 'Es sind noch keine Bot-Konfigurationen registriert.'; return; }
        selector.value = state.bot.id; note.textContent = state.bot.note;
        if (state.bot.available) await load();
        else status.textContent = 'Die TOML-Verbindung ist noch nicht eingerichtet.';
      });
      return () => { state.disposed = true; state.epoch += 1; stopPolling(); window.removeEventListener('beforeunload', beforeUnload); root.replaceChildren(); };
    }
  };
})();
