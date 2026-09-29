/* Vollständiger Katalog; Anmeldung, Origin und CSRF bleiben im vorhandenen fetchJSON. */
(() => {
    'use strict';
    const V41 = 'accounts/fireworks/models/deepseek-v4p1-flash';
    const definitions = [
        { key: 'discord', title: 'Discord', endpoint: '/api/admin/betriebskonfiguration' },
        { key: 'steam', title: 'Steam', endpoint: '/api/admin/steam-betriebskonfiguration' },
    ];
    const mounted = new Map();
    const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    const displayValue = value => value === null ? 'Dienststandard' : value === true ? 'An' : value === false ? 'Aus' : Array.isArray(value) ? (value.join(', ') || 'Leere Liste') : String(value);
    function parseValue(field, raw) {
        if (raw === null) {
            if (!field.nullable) throw new Error('Dieses Feld braucht einen Wert.');
            return null;
        }
        if (field.kind === 'boolean') {
            if (raw !== true && raw !== false) throw new Error('Bitte An, Aus oder Dienststandard wählen.');
            return raw;
        }
        if (field.kind === 'integer') {
            const value = String(raw).trim();
            if (!/^(0|-?[1-9][0-9]*)$/.test(value)) throw new Error('Bitte eine ganze Zahl ohne Dezimalstellen eingeben.');
            const number = BigInt(value);
            if ((field.min !== null && number < BigInt(field.min)) || (field.max !== null && number > BigInt(field.max))) {
                throw new Error(`Erlaubt sind ganze Zahlen von ${field.min} bis ${field.max}.`);
            }
            return value; // IDs niemals durch Number() runden.
        }
        if (field.kind === 'number') {
            if (String(raw).trim() === '') throw new Error('Bitte eine Zahl eingeben.');
            const value = Number(raw);
            if (!Number.isFinite(value)) throw new Error('Bitte eine gültige Zahl eingeben.');
            if ((field.min !== null && value < Number(field.min)) || (field.max !== null && value > Number(field.max))) throw new Error(`Erlaubt sind Werte von ${field.min} bis ${field.max}.`);
            return value;
        }
        if (field.kind === 'integer_list' || field.kind === 'string_list') {
            const parts = Array.isArray(raw) ? raw : String(raw).split(field.kind === 'integer_list' ? /[\n,]/ : /\n/).map(value => value.trim()).filter(Boolean);
            if (parts.length > 512 || (field.length !== null && parts.length !== field.length)) {
                throw new Error(field.length !== null ? `Diese Liste braucht genau ${field.length} Einträge.` : 'Höchstens 512 Einträge.');
            }
            return parts.map(value => parseValue({ ...field, kind: field.kind === 'integer_list' ? 'integer' : 'string', nullable: false }, value));
        }
        const value = String(raw);
        if (new TextEncoder().encode(value).length > 2048 || /[\u0000-\u001f\u007f-\u009f]/.test(value)) throw new Error('Der Text ist zu lang oder enthält Steuerzeichen.');
        if (field.kind === 'choice' && !field.choices.includes(value)) throw new Error('Bitte einen angebotenen Wert auswählen.');
        return value;
    }
    function changesForV41(values, fields) {
        const changes = { 'llm.fireworks.model': V41 };
        for (const field of fields) {
            const match = /^llm\.use_cases\.([a-z_]+)\.model$/.exec(field.path);
            if (!match || !field.writable) continue;
            const name = match[1];
            const fallback = ['moderation_verify', 'turnier_vorschlag', 'voice_hint'].includes(name) ? 'openai' : 'fireworks';
            const provider = values[`llm.use_cases.${name}.provider`] ?? values['llm.default_provider'] ?? fallback;
            if (provider === 'fireworks') changes[field.path] = V41;
        }
        return changes;
    }
    // Kleine reine Funktionen sind ohne Browser und ohne API testbar.
    if (typeof module !== 'undefined' && module.exports) module.exports = { parseValue, changesForV41, equal, V41 };
    if (typeof document === 'undefined') return;
    function element(tag, text, className) {
        const node = document.createElement(tag);
        if (text !== undefined) node.textContent = text;
        if (className) node.className = className;
        return node;
    }
    function button(text, action) {
        const node = element('button', text); node.type = 'button'; node.addEventListener('click', action); return node;
    }
    function styles() {
        if (document.getElementById('operating-settings-style')) return;
        const node = element('style'); node.id = 'operating-settings-style';
        node.textContent = `
        #operating-config-cards .settings-service {padding:22px;background:#101113;border:1px solid #66532d;border-radius:10px;box-shadow:0 8px 25px #0006;margin-bottom:24px;}
        .settings-service {font-size:14px;line-height:1.5;min-width:0;}
        .settings-service h2 {margin:0;font-size:22px;}
        .settings-header,.settings-tools,.settings-actions {display:flex;align-items:center;gap:14px;flex-wrap:wrap;}
        .settings-header {justify-content:space-between;margin-bottom:12px;}
        .settings-count,.settings-hint,.settings-path {color:#b6b2a8;font-size:12px;}
        .settings-status {display:grid;gap:5px;margin:12px 0 18px;}
        .settings-status span {border-left:2px solid #887449;padding-left:10px;}
        .settings-error {color:#ffb3a3;white-space:pre-wrap;}
        .settings-error:empty {display:none;}
        .settings-tools {margin:16px 0;}
        .settings-search {flex:1;min-width:min(100%,240px);}
        .settings-service input:not([type=checkbox]),.settings-service select,.settings-service textarea {box-sizing:border-box;width:100%;font:inherit;color:#f6f2e9;background:#08090b;border:1px solid #555047;border-radius:5px;padding:9px 11px;min-height:40px;}
        .settings-service input:focus,.settings-service select:focus,.settings-service textarea:focus {outline:2px solid #d0ae63;outline-offset:2px;}
        .settings-service :disabled {opacity:.55;}
        .settings-service fieldset {border:0;padding:0;margin:0;min-width:0;}
        .settings-group {border:1px solid #3e3930;border-radius:6px;margin:12px 0;overflow:hidden;}
        .settings-group>summary {cursor:pointer;padding:13px 16px;font-weight:700;color:#ead7a2;background:#171719;}
        .settings-fields {display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,320px),1fr));gap:18px;padding:18px;}
        .settings-field {min-width:0;padding:10px;border-left:2px solid transparent;}
        .settings-field.is-changed {border-left-color:#d0ae63;background:#d0ae6309;}
        .settings-label {display:block;font-weight:600;font-size:14px;color:#f3efe5;margin-bottom:7px;}
        .settings-field .settings-hint {margin:8px 0 0;}
        .settings-path {overflow-wrap:anywhere;margin:6px 0 0;font-family:monospace;font-size:11px;}
        .settings-override {display:flex;gap:7px;align-items:center;margin-bottom:7px;font-size:12px;}
        .settings-actions {position:sticky;bottom:0;z-index:2;padding:14px;background:#141414;border:1px solid #6e5b33;border-radius:6px;margin-top:16px;box-shadow:0 -8px 20px #0005;}
        .settings-actions button,.settings-model-action {padding:10px 16px;min-height:40px;}
        .settings-diff {margin:16px 0;border-top:1px solid #544731;padding-top:12px;}
        .settings-diff li {margin:10px 0;overflow-wrap:anywhere;}
        .settings-diff code {font-size:12px;white-space:pre-wrap;}
        .settings-empty {padding:16px;color:#b6b2a8;}
        .settings-field[hidden],.settings-group[hidden] {display:none!important;}
        @media(max-width:640px){#operating-config-cards .settings-service{padding:14px}.settings-fields{padding:8px;gap:8px}.settings-actions{position:static}.settings-tools{align-items:stretch;flex-direction:column}}
        `;
        document.head.append(node);
    }
    function mount(definition) {
        const root = document.getElementById('operating-config-cards');
        const card = element('section', undefined, 'card settings-service');
        card.dataset.service = definition.key;
        const heading = element('div', undefined, 'settings-header');
        const count = element('span', 'Einstellungen werden geladen …', 'settings-count');
        heading.append(element('h2', definition.title), count);
        const status = element('div', undefined, 'settings-status'); status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
        const error = element('p', undefined, 'settings-error'); error.setAttribute('role', 'alert');
        const form = element('form'); form.noValidate = true;
        const tools = element('div', undefined, 'settings-tools');
        const search = element('input', undefined, 'settings-search'); search.type = 'search'; search.placeholder = 'Einstellung suchen: Modell, Voice, Rolle, Timeout …'; search.setAttribute('aria-label', `${definition.title}-Einstellungen durchsuchen`);
        function filterControl(text) { const label = element('label'); const input = element('input'); input.type = 'checkbox'; label.append(input, document.createTextNode(` ${text}`)); tools.append(label); return input; }
        tools.append(search);
        const changedOnly = filterControl('Nur geändert'); const protectedToo = filterControl('Geschützte Infrastruktur anzeigen');
        const fieldset = element('fieldset');
        const empty = element('p', 'Keine passenden Einstellungen.', 'settings-empty'); empty.hidden = true;
        const diff = element('details', undefined, 'settings-diff'); diff.hidden = true;
        const diffTitle = element('summary'); const diffList = element('ul'); diff.append(diffTitle, diffList);
        const actions = element('div', undefined, 'settings-actions');
        const submit = element('button', 'Änderungen speichern'); submit.type = 'submit'; submit.disabled = true;
        const reload = button('Stand neu laden', () => { void load(true); });
        const draftStatus = element('span', 'Keine ungespeicherten Änderungen', 'settings-count');
        actions.append(submit, reload, draftStatus);
        form.append(tools, fieldset, empty, diff, element('p', 'Speichern ändert die Betriebsdatei, startet aber keinen Dienst neu. „Dienststandard“ entfernt einen optionalen Override; es ist keine Aussage über den gerade laufenden Wert.', 'settings-hint'), actions);
        card.append(heading, status, error, form); root.append(card);
        const state = { loaded: null, entries: new Map(), groups: [], dirty: false, busy: false };
        function read(entry) {
            const { field, control, override } = entry;
            const raw = override && !override.checked ? null : field.kind === 'boolean' ? (field.nullable ? control.value === '' ? null : control.value === 'true' : control.checked) : field.kind === 'choice' && field.nullable && control.value === '' ? null : control.value;
            control.setCustomValidity('');
            try { return parseValue(field, raw); }
            catch (problem) { control.setCustomValidity(problem.message); throw problem; }
        }
        function set(entry, value) {
            const { field, control, override } = entry;
            if (override) { override.checked = value !== null; control.disabled = value === null; }
            if (field.kind === 'boolean' && !field.nullable) control.checked = value === true;
            else control.value = value === null ? '' : Array.isArray(value) ? value.join('\n') : String(value);
        }
        function filter() {
            const query = search.value.trim().toLocaleLowerCase('de'); let visible = 0;
            for (const group of state.groups) {
                let groupVisible = 0;
                for (const entry of group.entries) {
                    const matches = (!query || entry.search.includes(query)) && (!changedOnly.checked || entry.changed) && (entry.field.writable || protectedToo.checked);
                    entry.wrapper.hidden = !matches;
                    if (matches) groupVisible++;
                }
                group.node.hidden = groupVisible === 0;
                if (query || changedOnly.checked) group.node.open = groupVisible > 0;
                visible += groupVisible;
            }
            empty.hidden = visible !== 0 || !state.loaded;
        }
        function draft() {
            let changed = 0; diffList.replaceChildren();
            for (const entry of state.entries.values()) {
                let value; let invalid = false;
                try { value = read(entry); } catch (_) { invalid = true; }
                entry.changed = invalid || !equal(value, state.loaded.catalog.values[entry.field.path]);
                entry.wrapper.classList.toggle('is-changed', entry.changed);
                if (!entry.changed) continue;
                changed++;
                const item = element('li');
                item.append(element('strong', `${entry.field.group}: ${entry.field.label}`), element('br'), element('code', `${displayValue(state.loaded.catalog.values[entry.field.path])} → ${invalid ? 'Eingabe noch ungültig' : displayValue(value)}`));
                diffList.append(item);
            }
            state.dirty = changed > 0; diff.hidden = !state.dirty;
            diffTitle.textContent = `${changed} Änderung${changed === 1 ? '' : 'en'} vor dem Speichern prüfen`;
            draftStatus.textContent = changed ? `${changed} ungespeicherte Änderung${changed === 1 ? '' : 'en'}` : 'Keine ungespeicherten Änderungen';
            submit.disabled = state.busy || !state.dirty;
            reload.textContent = state.dirty ? 'Entwurf verwerfen und neu laden' : 'Stand neu laden'; filter();
        }
        function busy(value) { state.busy = value; fieldset.disabled = value; reload.disabled = value; submit.disabled = value || !state.dirty; }
        function render(data) {
            if (!data.catalog || data.catalog.version !== 1 || !Array.isArray(data.catalog.fields) || !data.catalog.values || !/^[a-f0-9]{64}$/i.test(data.revision)) {
                throw new Error('Dieser Dienst liefert noch keinen vollständigen Einstellungskatalog. Die Dienstversion muss aktualisiert werden. Vorhandene Entwürfe bleiben erhalten.');
            }
            const reportedServices = definition.key === 'discord' ? data.services : data.active;
            if (!Array.isArray(reportedServices) || data.catalog.fields.some(field => field.writable && !Object.hasOwn(data.catalog.values, field.path))) {
                throw new Error('Die Dienstantwort ist unvollständig. Dein vorhandener Entwurf bleibt erhalten.');
            }
            const services = reportedServices.map(item => {
                if (definition.key === 'discord') {
                    if (typeof item.name !== 'string' || ![true, false, null].includes(item.restart_required)) throw new Error('Ungültiger Dienststatus. Dein vorhandener Entwurf bleibt erhalten.');
                    return item;
                }
                if (typeof item.service !== 'string' || !['unknown', 'restart_required', 'current'].includes(item.state)) throw new Error('Ungültiger Dienststatus. Dein vorhandener Entwurf bleibt erhalten.');
                return { name: item.service === 'steam-bot' ? 'Steam-Bot' : item.service.replace('steam-core-', 'Steam-Konto '), restart_required: item.state === 'unknown' ? null : item.state === 'restart_required' };
            });
            state.loaded = data; state.entries = new Map(); state.groups = []; fieldset.replaceChildren();
            const editable = data.catalog.fields.filter(field => field.writable).length;
            count.textContent = `${editable} änderbar · ${data.catalog.fields.length - editable} geschützt`;
            status.replaceChildren(...services.map(item => element('span', `${item.name}: ${item.restart_required === null ? 'Aktiver Stand nicht bestätigt' : item.restart_required ? 'Gespeichert, Neustart noch ausstehend' : 'Gespeicherter Stand ist aktiv'}`)));
            if (definition.key === 'discord') {
                const preset = button('DeepSeek V4.1 Flash für Fireworks wählen', () => {
                    try {
                        const current = Object.fromEntries([...state.entries].map(([path, entry]) => [path, read(entry)]));
                        for (const [path, value] of Object.entries(changesForV41(current, data.catalog.fields))) {
                            const entry = state.entries.get(path); if (entry) set(entry, value);
                        }
                        draft(); diff.open = true; error.textContent = '';
                    } catch (problem) { error.textContent = `Zuerst ungültige Eingaben korrigieren: ${problem.message}`; }
                });
                preset.className = 'settings-model-action';
                fieldset.append(preset, element('p', 'Setzt den Fireworks-Standard und die Modell-Pins aller derzeit Fireworks zugeordneten KI-Funktionen. OpenAI-Zuordnungen bleiben unverändert. Erst „Änderungen speichern“ übernimmt die Auswahl in die Betriebsdatei.', 'settings-hint'));
            }
            const byGroup = new Map();
            for (const field of data.catalog.fields) { if (!byGroup.has(field.group)) byGroup.set(field.group, []); byGroup.get(field.group).push(field); }
            const groupOrder = name => name === 'KI · Standard' ? 0 : name.startsWith('KI') ? 1 : 2;
            const ordered = [...byGroup].sort(([a], [b]) => groupOrder(a) - groupOrder(b) || a.localeCompare(b, 'de'));
            for (const [name, fields] of ordered) {
                const group = element('details', undefined, 'settings-group'); group.open = name === 'KI · Standard';
                group.append(element('summary', `${name} · ${fields.length}`));
                const grid = element('div', undefined, 'settings-fields'); group.append(grid); fieldset.append(group);
                const groupState = { node: group, entries: [] }; state.groups.push(groupState);
                for (const field of fields) {
                    const wrapper = element('div', undefined, 'settings-field');
                    const id = `setting-${definition.key}-${field.path.replaceAll('.', '-')}`;
                    const caption = element('label', field.label, 'settings-label'); caption.htmlFor = id; wrapper.append(caption);
                    const entry = { field, wrapper, changed: false, search: `${name} ${field.label} ${field.path} ${field.help}`.toLocaleLowerCase('de') };
                    groupState.entries.push(entry); grid.append(wrapper);
                    if (!field.writable) {
                        caption.removeAttribute('for'); wrapper.append(element('p', field.help, 'settings-hint'), element('p', field.path, 'settings-path')); continue;
                    }
                    let control; let override = null;
                    if (field.kind === 'choice' || (field.kind === 'boolean' && field.nullable)) {
                        control = element('select');
                        const options = field.kind === 'boolean' ? [['true', 'An'], ['false', 'Aus']] : field.choices.map(value => [value, value === 'fireworks' ? 'Fireworks' : value === 'openai' ? 'OpenAI' : value]);
                        if (field.nullable) options.unshift(['', 'Dienststandard']);
                        for (const [value, text] of options) { const option = element('option', text); option.value = value; control.append(option); }
                    } else if (field.kind.endsWith('_list')) { control = element('textarea'); control.rows = 3; control.placeholder = 'Ein Eintrag pro Zeile'; }
                    else { control = element('input'); control.type = field.kind === 'boolean' ? 'checkbox' : field.kind === 'number' ? 'number' : 'text'; if (field.kind === 'number') control.step = 'any'; if (field.kind === 'integer') control.inputMode = 'numeric'; }
                    control.id = id; control.name = field.path;
                    if (field.nullable && field.kind !== 'boolean' && field.kind !== 'choice') {
                        const label = element('label', undefined, 'settings-override'); override = element('input'); override.type = 'checkbox'; override.setAttribute('aria-label', `${field.label}: eigenen Wert verwenden`); label.append(override, document.createTextNode('Eigenen Wert verwenden')); wrapper.append(label);
                        override.addEventListener('change', () => { control.disabled = !override.checked; draft(); });
                    }
                    entry.control = control; entry.override = override;
                    const hint = element('p', field.help, 'settings-hint'); hint.id = `${id}-help`; control.setAttribute('aria-describedby', hint.id);
                    wrapper.append(control, hint, element('p', field.path, 'settings-path'));
                    set(entry, data.catalog.values[field.path] ?? null);
                    control.addEventListener('input', draft); control.addEventListener('change', draft); state.entries.set(field.path, entry);
                }
            }
            draft();
        }
        async function load(confirmDiscard) {
            if (state.busy || (confirmDiscard && state.dirty && !window.confirm('Ungespeicherte Änderungen wirklich verwerfen und den aktuellen Stand laden?'))) return;
            busy(true); error.textContent = '';
            try { render(await fetchJSON(definition.endpoint, { cache: 'no-store' })); }
            catch (problem) { error.textContent = problem.message || 'Einstellungen konnten nicht geladen werden.'; }
            finally { busy(false); }
        }
        search.addEventListener('input', filter); changedOnly.addEventListener('change', filter); protectedToo.addEventListener('change', filter);
        form.addEventListener('submit', async event => {
            event.preventDefault(); if (!state.loaded || !state.dirty || state.busy) return;
            const changes = Object.create(null);
            try {
                for (const [path, entry] of state.entries) { const value = read(entry); if (!equal(value, state.loaded.catalog.values[path])) changes[path] = value; }
            } catch (problem) {
                error.textContent = problem.message; for (const entry of state.entries.values()) if (!entry.control.checkValidity()) { entry.wrapper.closest('details').open = true; entry.wrapper.hidden = false; entry.wrapper.closest('details').hidden = false; entry.control.reportValidity(); break; } return;
            }
            if (!Object.keys(changes).length) return;
            busy(true); error.textContent = '';
            try {
                const data = await fetchJSON(definition.endpoint, { method: 'PATCH', body: JSON.stringify({ revision: state.loaded.revision, changes }) });
                render(data); status.prepend(element('span', 'Änderungen gespeichert. Kein Dienst wurde automatisch neu gestartet.'));
            } catch (problem) { error.textContent = `${problem.message || 'Speichern fehlgeschlagen.'} Dein Entwurf bleibt erhalten.`; }
            finally { busy(false); }
        });
        return { load: () => load(false), dirty: () => state.dirty };
    }
    window.loadOperatingConfig = () => {
        if (!document.getElementById('operating-config-cards')) return; styles();
        for (const definition of definitions) if (!mounted.has(definition.key)) {
            const view = mount(definition); mounted.set(definition.key, view); void view.load();
        }
    };
    window.addEventListener('beforeunload', event => { if ([...mounted.values()].some(view => view.dirty())) { event.preventDefault(); event.returnValue = ''; } });
    if (location.hash === '#betrieb') window.loadOperatingConfig();
})();
