/* Bestehende Dashboard-Anmeldung und fetchJSON verwenden. Keine Diensttokens. */
(() => {
    const definitions = [
        { key: 'discord', title: 'Discord', endpoint: '/api/admin/betriebskonfiguration', fields: [
            ['moderation_enforce', 'Moderationsmaßnahmen anwenden', 'checkbox'],
            ['concierge_timeout_seconds', 'Antwortzeitlimit des Bot-Paten (Sekunden)', 'number', 1, 110],
        ] },
        { key: 'steam', title: 'Steam', endpoint: '/api/admin/steam-betriebskonfiguration', fields: [
            ['friends.slot_limit', 'Freundesplätze insgesamt', 'number', 1, 32767],
            ['friends.slot_reserve', 'Freie Plätze als Reserve', 'number', 0, 32767],
            ['friends.remove_timeout_secs', 'Zeitlimit beim Entfernen (Sekunden)', 'number', 1, 3600],
            ['core.presence_interval_secs', 'Abstand der Statusprüfung (Sekunden)', 'number', 1, 86400],
            ['core.presence_chunk_size', 'Konten pro Statusabfrage', 'number', 1, 100],
            ['core.presence_chunk_delay_ms', 'Pause zwischen Statusabfragen (Millisekunden)', 'number', 1, 60000],
            ['rank.batch_size', 'Ränge pro Abfrage', 'number', 1, 100],
        ] },
    ];
    const mounted = new Map();
    function element(tag, text, className) {
        const node = document.createElement(tag);
        if (text !== undefined) node.textContent = text;
        if (className) node.className = className;
        return node;
    }
    function mount(definition) {
        const root = document.getElementById('operating-config-cards');
        const card = element('section', undefined, 'card');
        card.append(element('h2', definition.title));
        const status = element('p', 'Gespeicherter und aktiver Stand werden geprüft …');
        status.setAttribute('role', 'status');
        const error = element('p'); error.setAttribute('role', 'alert');
        const form = element('form');
        const fieldset = element('fieldset');
        const submit = element('button', 'Änderungen speichern'); submit.type = 'submit'; submit.disabled = true;
        const reload = element('button', 'Stand neu laden'); reload.type = 'button';
        card.append(status, error, form, reload);
        form.append(fieldset, element('p', 'Speichern startet keinen Dienst neu. Geänderte Werte werden erst nach dem Neustart übernommen.'), submit);
        root.append(card);
        const state = { loaded: null, dirty: false, busy: false, inputs: [] };
        function busy(value) {
            state.busy = value; fieldset.disabled = value; reload.disabled = value;
            submit.disabled = value || !state.dirty;
        }
        function input(path, label, type, min, max, value) {
            const id = `operating-${definition.key}-${path.replaceAll('.', '-')}`;
            const wrapper = element('div', undefined, 'form-group');
            const caption = element('label', label); caption.htmlFor = id;
            const control = element('input'); control.id = id; control.type = type;
            if (type === 'checkbox') control.checked = Boolean(value);
            else { control.required = true; control.step = '1'; control.min = String(min); control.max = String(max); control.value = String(value); }
            control.addEventListener('input', () => {
                state.dirty = true; submit.disabled = state.busy; reload.textContent = 'Entwurf verwerfen und neu laden';
            });
            wrapper.append(caption, control); fieldset.append(wrapper);
            state.inputs.push({ path, control });
        }
        function render(data) {
            state.loaded = data; state.dirty = false; state.inputs = []; fieldset.replaceChildren();
            const settings = definition.key === 'discord' ? data.options : data.editable;
            for (const [path, label, type, min, max] of definition.fields) {
                const value = path.split('.').reduce((current, key) => current[key], settings);
                input(path, label, type, min, max, value);
            }
            if (definition.key === 'steam') for (const account of settings.accounts) {
                const label = `Steam-Konto ${account.id}`;
                fieldset.append(element('h3', label));
                input(`accounts.${account.id}.presence_enabled`, `${label}: Spielstatus prüfen`, 'checkbox', null, null, account.presence_enabled);
                input(`accounts.${account.id}.catalog_maintenance_enabled`, `${label}: Katalog pflegen`, 'checkbox', null, null, account.catalog_maintenance_enabled);
            }
            if (definition.key === 'steam') fieldset.append(element('p', 'Höchstens ein Konto darf den Katalog pflegen. Bei einem Wechsel der Katalogwartung zuerst beide Steam-Cores stoppen, danach beide starten.'));
            const services = definition.key === 'discord' ? data.services : data.active.map(item => ({
                name: item.service === 'steam-bot' ? 'Steam-Bot' : item.service.replace('steam-core-', 'Steam-Konto '),
                restart_required: item.state === 'unknown' ? null : item.state === 'restart_required',
            }));
            status.textContent = services.map(item => `${item.name}: ${item.restart_required === null ? 'aktiver Stand nicht bestätigt' : item.restart_required ? 'Neustart ausstehend' : 'gespeicherter Stand aktiv'}`).join(' · ');
            reload.textContent = 'Stand neu laden'; submit.disabled = true;
        }
        async function load() {
            if (state.busy) return;
            busy(true); error.textContent = '';
            try { render(await fetchJSON(definition.endpoint, { cache: 'no-store' })); }
            catch (problem) { error.textContent = problem.message; }
            finally { busy(false); }
        }
        reload.addEventListener('click', load);
        form.addEventListener('submit', async event => {
            event.preventDefault();
            if (!state.loaded || !state.dirty || state.busy || !form.reportValidity()) return;
            const values = {};
            for (const { path, control } of state.inputs) {
                const parts = path.split('.'); const value = control.type === 'checkbox' ? control.checked : Number(control.value);
                if (parts[0] === 'accounts') {
                    values.accounts ??= [];
                    let account = values.accounts.find(item => item.id === Number(parts[1]));
                    if (!account) { account = { id: Number(parts[1]) }; values.accounts.push(account); }
                    account[parts[2]] = value;
                } else if (parts.length === 1) values[path] = value;
                else { values[parts[0]] ??= {}; values[parts[0]][parts[1]] = value; }
            }
            busy(true); error.textContent = '';
            try {
                const body = { revision: state.loaded.revision, [definition.key === 'discord' ? 'options' : 'patch']: values };
                render(await fetchJSON(definition.endpoint, { method: 'PATCH', body: JSON.stringify(body) }));
                status.textContent = `Gespeichert. ${status.textContent}`;
            } catch (problem) { error.textContent = problem.message; }
            finally { busy(false); }
        });
        return { load };
    }
    window.loadOperatingConfig = () => {
        for (const definition of definitions) if (!mounted.has(definition.key)) {
            const view = mount(definition); mounted.set(definition.key, view); void view.load();
        }
    };
    if (location.hash === '#betrieb') window.loadOperatingConfig();
})();
