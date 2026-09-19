(function () {
    'use strict';
    const byId = id => document.getElementById(id);
    const ui = {
        search: byId('brain-map-search'), filter: byId('brain-map-filter'),
        results: byId('brain-map-results'), count: byId('brain-map-count'),
        status: byId('brain-map-status'), error: byId('brain-map-error'),
        layout: byId('brain-map-layout'), summary: byId('brain-map-summary'),
        title: byId('brain-map-title'), meta: byId('brain-map-meta'),
        source: byId('brain-map-source'), evidence: byId('brain-map-evidence'),
        links: byId('brain-map-links'), canvas: byId('brain-map-canvas'),
        scope: byId('brain-map-scope'), more: byId('brain-map-more'),
    };
    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const relationLabels = {documents: 'belegt', calls: 'ruft auf', contains: 'enthält', defines: 'definiert', imports: 'importiert', imports_from: 'importiert aus', uses: 'nutzt', references: 'verweist auf', depends_on: 'hängt ab von', rationale_for: 'begründet', method: 'Methode'};
    let graph = null;
    let nodes = new Map();
    let neighbors = new Map();
    let network = null;
    let selected = null;
    let pageSize = 60;
    let loading = false;
    let libraryPromise = null;

    function loadLibrary() {
        if (window.vis?.Network) return Promise.resolve();
        if (libraryPromise) return libraryPromise;
        libraryPromise = new Promise((resolve, reject) => {
            const script = document.createElement('script');
            const timer = setTimeout(() => { script.remove(); reject(new Error('Die Graphanzeige lädt zu lange. Bitte erneut versuchen.')); }, 15000);
            script.src = '/api/brain/graph-library.js';
            script.onload = () => { clearTimeout(timer); window.vis?.Network ? resolve() : reject(new Error('Die Graphanzeige ist nicht verfügbar.')); };
            script.onerror = () => { clearTimeout(timer); script.remove(); reject(new Error('Die Graphanzeige ist nicht verfügbar. Bitte erneut anmelden oder später versuchen.')); };
            document.head.append(script);
        }).catch(error => { libraryPromise = null; throw error; });
        return libraryPromise;
    }

    function matches(node) {
        const query = ui.search.value.trim().toLocaleLowerCase('de');
        const text = [node.label, node.source_file, node.source_location, node.group].join(' ').toLocaleLowerCase('de');
        if (query && !text.includes(query)) return false;
        if (ui.filter.value === 'documents') return node.kind === 'document';
        if (ui.filter.value === 'code') return node.kind === 'code';
        if (ui.filter.value === 'discord') return /discord|concierge|onboarding|pate|dl-community/.test(text);
        return true;
    }

    function showResults() {
        if (!graph) return;
        const matchesList = graph.nodes.filter(matches).sort((a, b) => (a.kind === 'document' ? 0 : 1) - (b.kind === 'document' ? 0 : 1) || a.label.localeCompare(b.label, 'de'));
        ui.count.textContent = matchesList.length + (matchesList.length === 1 ? ' Treffer' : ' Treffer');
        ui.results.replaceChildren();
        for (const node of matchesList.slice(0, pageSize)) {
            const button = document.createElement('button');
            button.type = 'button';
            button.dataset.nodeId = node.id;
            button.setAttribute('aria-pressed', String(node.id === selected));
            const name = document.createElement('span');
            name.textContent = node.label || node.id;
            const hint = document.createElement('small');
            hint.textContent = node.kind === 'document' ? (node.source_file.startsWith('public/') ? 'Öffentliche Quelle · ' : 'Interne Dokumentation · ') + (node.group || 'Dokument') : 'Code · ' + node.source_file;
            button.append(name, hint);
            button.addEventListener('click', () => select(node.id));
            ui.results.append(button);
        }
        if (!matchesList.length) ui.results.textContent = 'Keine Treffer. Versuche einen anderen Begriff oder Bereich.';
        ui.more.hidden = matchesList.length <= pageSize;
        ui.more.textContent = 'Weitere Treffer (' + Math.max(0, matchesList.length - pageSize) + ')';
    }

    function draw(node, incident) {
        if (!window.vis?.Network) return;
        const ids = [node.id, ...new Set(incident.map(link => link.source === node.id ? link.target : link.source))].slice(0, 90);
        const visible = new Set(ids);
        const links = incident.filter(link => visible.has(link.source) && visible.has(link.target)).slice(0, 400);
        const data = {
            nodes: ids.map((id, index) => {
                const entry = nodes.get(id);
                const angle = (index - 1) * 2 * Math.PI / Math.max(1, ids.length - 1);
                const fullLabel = entry.label || id;
                const shortLabel = fullLabel.length > 48 ? fullLabel.slice(0, 45) + '…' : fullLabel;
                return {id, label: id === node.id || ids.length <= 20 ? shortLabel : '', shape: entry.kind === 'document' ? 'dot' : 'diamond', size: id === node.id ? 22 : 12,
                    color: {background: entry.kind === 'document' ? '#d6a62c' : '#986d34', border: id === node.id ? '#fff3ca' : '#f0cc78'},
                    font: {color: '#fff1d5', size: 13}, x: index ? 230 * Math.cos(angle) : 0, y: index ? 230 * Math.sin(angle) : 0};
            }),
            edges: links.map((link, index) => ({id: index, from: link.source, to: link.target, arrows: 'to', color: link.relation === 'documents' ? '#d6a62c' : '#735225', dashes: link.confidence === 'INFERRED', width: link.relation === 'documents' ? 2 : 1})),
        };
        if (network) network.destroy();
        network = new window.vis.Network(ui.canvas, data, {
            autoResize: true,
            physics: reducedMotion ? false : {stabilization: {iterations: 80, fit: true}, solver: 'barnesHut', barnesHut: {gravitationalConstant: -2200, springLength: 140}},
            interaction: {hover: true, keyboard: false, zoomView: true, dragView: true},
            edges: {smooth: false},
        });
        network.on('click', params => { if (params.nodes.length) select(params.nodes[0]); });
        if (ids.length > 20) {
            network.on('hoverNode', params => { const entry = nodes.get(params.node); if (entry && params.node !== node.id) network.body.data.nodes.update({id: params.node, label: entry.label || entry.id}); });
            network.on('blurNode', params => { if (params.node !== node.id) network.body.data.nodes.update({id: params.node, label: ''}); });
        }
        network.once('stabilizationIterationsDone', () => network?.setOptions({physics: false}));
        network.fit({animation: false});
        ui.scope.textContent = ids.length + ' Knoten in der direkten Nachbarschaft. ' + (incident.length > links.length ? 'Weitere Verbindungen stehen vollständig in der Liste rechts. ' : '') + 'Gold: Wissensdokument · Bronze: Code. Ziehen und Zoomen bewegt die Karte; Treffer und Verbindungen sind auch per Tastatur bedienbar.';
    }

    function select(id) {
        const node = nodes.get(id);
        if (!node) return;
        selected = id;
        for (const button of ui.results.querySelectorAll('button[data-node-id]')) button.setAttribute('aria-pressed', String(button.dataset.nodeId === id));
        ui.title.textContent = node.label || node.id;
        ui.meta.textContent = [node.kind === 'document' ? (node.source_file.startsWith('public/') ? 'Öffentliche Wissensquelle' : 'Interne Dokumentation · kein Concierge-Antwortwissen') : 'Code', node.group, node.stand ? 'Dokumentstand: ' + node.stand : ''].filter(Boolean).join(' · ');
        ui.source.value = [node.source_file, node.source_location].filter(Boolean).join(':');
        const incident = neighbors.get(id) || [];
        const documents = incident.filter(link => link.relation === 'documents');
        ui.evidence.textContent = node.kind === 'document'
            ? documents.length ? documents.length + ' belegte Code-Verbindungen. Die Linien stammen aus der vorhandenen Wissenskarte.' : 'Für dieses Dokument ist noch keine Code-Verbindung hinterlegt.'
            : 'Verbindungen aus der bestehenden Codeanalyse. Abgeleitete Zusammenhänge sind als solche markiert.';
        ui.links.replaceChildren();
        const heading = document.createElement('h5');
        heading.textContent = 'Verbindungen (' + incident.length + ')';
        ui.links.append(heading);
        for (const link of incident) {
            const target = nodes.get(link.source === id ? link.target : link.source);
            if (!target) continue;
            const button = document.createElement('button');
            button.type = 'button';
            const direction = link.source === id ? '→' : '←';
            button.textContent = direction + ' ' + (relationLabels[link.relation] || link.relation || 'Verbindung') + ': ' + (target.label || target.id) + (link.confidence === 'INFERRED' ? ' (abgeleitet)' : '');
            button.addEventListener('click', () => { select(target.id); ui.source.focus(); });
            ui.links.append(button);
        }
        if (!incident.length) {
            const empty = document.createElement('p');
            empty.textContent = 'Noch keine Verbindungen in dieser Karte.';
            ui.links.append(empty);
        }
        ui.status.textContent = 'Ausgewählt: ' + (node.label || node.id) + '. ' + incident.length + ' Verbindungen.';
        draw(node, incident);
    }

    async function load(force = false) {
        if (loading || (graph && !force)) return;
        loading = true;
        ui.error.hidden = true;
        ui.status.textContent = 'Wissenskarte wird geladen …';
        byId('brain-map-reload').disabled = true;
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), 30000);
        try {
            const response = await fetch('/api/brain/graph', {credentials: 'same-origin', signal: controller.signal});
            if (!response.ok) throw new Error(response.status === 401 ? 'Bitte melde dich erneut an, um die Wissenskarte zu öffnen.' : response.status === 403 ? 'Die Wissenskarte ist nur mit vollem Admin-Zugriff verfügbar.' : 'Die Wissenskarte ist gerade nicht verfügbar. Bitte später erneut laden.');
            const data = await response.json();
            if (!Array.isArray(data.nodes) || !Array.isArray(data.links) || !data.nodes.length) throw new Error('Die Wissenskarte enthält noch keine Dokumente.');
            graph = data;
            nodes = new Map(data.nodes.map(node => [node.id, node]));
            neighbors = new Map(data.nodes.map(node => [node.id, []]));
            for (const link of data.links) {
                if (!nodes.has(link.source) || !nodes.has(link.target)) continue;
                neighbors.get(link.source).push(link);
                if (link.source !== link.target) neighbors.get(link.target).push(link);
            }
            const stamp = new Date(data.generated_at).toLocaleString('de-DE');
            ui.summary.textContent = data.corpus_nodes + ' Wissensdokumente · ' + data.nodes.length + ' Knoten · ' + data.links.length + ' Verbindungen. Ausschnitt: Wissensbasis, belegte Code-Bezüge und deren direkte Nachbarn. ' + data.unlinked_documents + ' Dokumente haben noch keine Code-Belege. Kartenstand: ' + stamp + '.' + (data.omitted_nodes || data.omitted_links ? ' Die Größe ist begrenzt; ' + data.omitted_nodes + ' weitere Nachbarn und ' + data.omitted_links + ' Verbindungen sind ausgeblendet.' : '');
            ui.layout.hidden = false;
            showResults();
            const initial = data.nodes.find(node => node.id === selected) || data.nodes.find(node => node.kind === 'document' && /concierge/.test(node.source_file) && (neighbors.get(node.id)?.length || 0) > 0) || data.nodes.find(node => node.kind === 'document' && (neighbors.get(node.id)?.length || 0) > 0) || data.nodes.find(node => node.kind === 'document') || data.nodes[0];
            select(initial.id);
            try { await loadLibrary(); const current = nodes.get(selected) || initial; draw(current, neighbors.get(current.id) || []); }
            catch (error) { ui.error.textContent = error.message + ' Suche und Verbindungsliste bleiben bedienbar.'; ui.error.hidden = false; }
        } catch (error) {
            graph = null;
            network?.destroy();
            network = null;
            ui.layout.hidden = true;
            ui.status.textContent = '';
            ui.error.textContent = error.name === 'AbortError' ? 'Das Laden dauert zu lange. Bitte versuche es erneut.' : error.message;
            ui.error.hidden = false;
        } finally {
            clearTimeout(timer);
            loading = false;
            byId('brain-map-reload').disabled = false;
        }
    }

    ui.search.addEventListener('input', () => { pageSize = 60; showResults(); });
    ui.filter.addEventListener('change', () => { pageSize = 60; showResults(); });
    ui.more.addEventListener('click', () => { const previous = pageSize; pageSize += 60; showResults(); ui.results.querySelectorAll('button')[previous]?.focus(); });
    byId('brain-map-reload').addEventListener('click', () => load(true));
    byId('brain-map-copy').addEventListener('click', async () => {
        try { await navigator.clipboard.writeText(ui.source.value); ui.status.textContent = 'Quellenpfad kopiert.'; }
        catch (_) { ui.source.focus(); ui.source.select(); ui.status.textContent = 'Quellenpfad markiert. Kopiere ihn mit Strg+C oder ⌘C.'; }
    });
    window.loadVisualBrain = load;
    if (document.querySelector('.tab-panel[data-tab="brain"].active')) load();
})();
