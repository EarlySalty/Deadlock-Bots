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
        knowledgeMode: byId('brain-map-mode-knowledge'), systemMode: byId('brain-map-mode-system'),
        detailClose: byId('brain-map-detail-close'), semanticSearch: byId('brain-map-semantic-search'),
        retrievalResults: byId('brain-map-retrieval-results'),
    };
    const relationLabels = {documents: 'belegt', shared_evidence: 'gemeinsamer Beleg', calls: 'ruft auf', contains: 'enthält', defines: 'definiert', imports: 'importiert', imports_from: 'importiert aus', uses: 'nutzt', references: 'verweist auf', depends_on: 'hängt ab von', rationale_for: 'begründet', method: 'Methode'};
    let graph = null;
    let nodes = new Map();
    let neighbors = new Map();
    let network = null;
    let selected = null;
    let pageSize = 60;
    let loading = false;
    let libraryPromise = null;
    let statusController = null;
    let statusGeneration = 0;
    let mapMode = 'knowledge';
    let view = {type: 'overview'};
    let topics = new Map();
    const topicNames = {'discord-server': 'Discord & Concierge', 'deadlock-helden': 'Deadlock · Helden', 'twitch-bot': 'Twitch', wissensbasis: 'Wissensbasis', dokus: 'Interne Dokumentation', 'patchnotes-bot': 'Patchnotes', 'steam-bot': 'Steam', 'index.html': 'Überblick', turniere: 'Turniere', website: 'Website'};

    function topicKey(node) {
        return node.kind === 'document' ? 'docs:' + (node.group || 'Dokumente') : 'code:' + (node.source_file.match(/rust\/(?:crates|bin)\/([^/]+)/)?.[1] || node.source_file.split('/')[0] || 'Weitere Quellen');
    }

    function topicName(key) {
        const value = key.slice(key.indexOf(':') + 1);
        return key.startsWith('docs:') ? (topicNames[value] || value) : value.split('/').slice(-2).join('/');
    }

    function defaultFilter() {
        return mapMode === 'knowledge' ? 'documents' : 'all';
    }

    function isKnowledgeNode(node) {
        return node?.layer ? node.layer === 'knowledge' : node?.kind === 'document';
    }

    function knowledgeNeighborhood(id) {
        const ids = new Set([id]);
        const evidenceTargets = new Set(
            (neighbors.get(id) || [])
                .filter(link => link.relation === 'documents' && link.source === id)
                .map(link => link.target)
        );
        if (!evidenceTargets.size) return ids;
        for (const link of graph.links) {
            if (link.relation !== 'documents' || !evidenceTargets.has(link.target)) continue;
            const source = nodes.get(link.source);
            if (isKnowledgeNode(source)) ids.add(link.source);
        }
        return ids;
    }

    function sharedKnowledgeEdges(ids, limit = 160) {
        const evidence = new Map();
        for (const link of graph.links) {
            if (link.relation !== 'documents' || !ids.has(link.source)) continue;
            const source = nodes.get(link.source);
            if (!isKnowledgeNode(source)) continue;
            if (!evidence.has(link.target)) evidence.set(link.target, []);
            evidence.get(link.target).push(link.source);
        }
        const pairs = new Map();
        for (const docs of evidence.values()) {
            const uniqueDocs = [...new Set(docs)].sort();
            for (let left = 0; left < uniqueDocs.length; left++) {
                for (let right = left + 1; right < uniqueDocs.length; right++) {
                    const key = uniqueDocs[left] + '\u0000' + uniqueDocs[right];
                    pairs.set(key, (pairs.get(key) || 0) + 1);
                    if (pairs.size >= limit) break;
                }
                if (pairs.size >= limit) break;
            }
            if (pairs.size >= limit) break;
        }
        return [...pairs].map(([key, count]) => {
            const [source, target] = key.split('\u0000');
            return {source, target, relation: 'shared_evidence', confidence: 'INFERRED', count};
        });
    }

    function wrap(value, width = 28) {
        return String(value).match(new RegExp('.{1,' + width + '}(?:\\s|$)|.{1,' + width + '}', 'g'))?.map(line => line.trim()).join('\n') || value;
    }

    function openTopic(key) {
        view = {type: 'topic', key};
        pageSize = 60;
        showResults();
        renderMap();
        ui.status.textContent = topicName(key) + ' geöffnet. Wähle einen Eintrag, um die Zusammenhänge zu erkunden.';
    }

    function overview() {
        selected = null;
        view = {type: 'overview'};
        ui.search.value = '';
        ui.filter.value = defaultFilter();
        showResults();
        renderMap();
    }

    function setMapMode(mode) {
        if (mode !== 'knowledge' && mode !== 'system') return;
        mapMode = mode;
        selected = null;
        view = {type: 'overview'};
        ui.search.value = '';
        ui.filter.value = defaultFilter();
        const codeOption = ui.filter.querySelector('option[value="code"]');
        const allOption = ui.filter.querySelector('option[value="all"]');
        if (codeOption) codeOption.disabled = mode === 'knowledge';
        if (allOption) allOption.textContent = mode === 'knowledge' ? 'Gesamtes Wissen' : 'Gesamter Graph';
        ui.knowledgeMode.setAttribute('aria-pressed', String(mode === 'knowledge'));
        ui.systemMode.setAttribute('aria-pressed', String(mode === 'system'));
        showResults();
        renderMap();
        ui.status.textContent = mode === 'knowledge'
            ? 'Wissensansicht aktiv. Technische Belege bleiben ausgeblendet, bis du zur Technikansicht wechselst.'
            : 'Technikansicht aktiv. Dokumente, Code und belegte Beziehungen werden gemeinsam dargestellt.';
    }

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
        if (mapMode === 'knowledge' && !isKnowledgeNode(node)) return false;
        if (query && !text.includes(query)) return false;
        if (view.type === 'topic' && topicKey(node) !== view.key) return false;
        if (view.type === 'node') {
            const visible = mapMode === 'knowledge'
                ? knowledgeNeighborhood(selected)
                : new Set([selected, ...(neighbors.get(selected) || []).flatMap(edge => [edge.source, edge.target])]);
            if (!visible.has(node.id)) return false;
        }
        if (ui.filter.value === 'documents') return isKnowledgeNode(node);
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
            button.addEventListener('click', () => { view = {type: 'node'}; ui.search.value = ''; ui.filter.value = defaultFilter(); select(node.id); showResults(); });
            ui.results.append(button);
        }
        if (!matchesList.length) ui.results.textContent = 'Keine Treffer. Versuche einen anderen Begriff oder Bereich.';
        ui.more.hidden = matchesList.length <= pageSize;
        ui.more.textContent = 'Weitere Treffer (' + Math.max(0, matchesList.length - pageSize) + ')';
    }

    async function semanticSearch() {
        const question = ui.search.value.trim();
        if (!question) {
            ui.search.focus();
            ui.status.textContent = 'Gib zuerst eine Suchfrage ein.';
            return;
        }
        ui.semanticSearch.disabled = true;
        ui.retrievalResults.hidden = false;
        ui.retrievalResults.replaceChildren();
        const loading = document.createElement('p');
        loading.textContent = 'Hybrid Retrieval durchsucht BM25, Vektorsuche und Reranking …';
        ui.retrievalResults.append(loading);
        ui.layout.classList.add('brain-results-open');
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), 6000);
        try {
            const response = await fetch('/api/brain/retrieve', {
                method: 'POST',
                credentials: 'same-origin',
                headers: {'content-type': 'application/json'},
                body: JSON.stringify({question}),
                signal: controller.signal
            });
            if (!response.ok) throw new Error('Die hybride Wissenssuche ist gerade nicht verfügbar.');
            const data = await response.json();
            if (!Array.isArray(data.evidence) || !['ready', 'no_evidence'].includes(data.status)) {
                throw new Error('Die Wissenssuche hat eine ungültige Antwort geliefert.');
            }
            ui.retrievalResults.replaceChildren();
            if (data.status === 'no_evidence' || !data.evidence.length) {
                const empty = document.createElement('p');
                empty.textContent = 'Nicht gefunden. Im freigegebenen Wissensstand gibt es keinen belegten Treffer.';
                ui.retrievalResults.append(empty);
                ui.status.textContent = 'Hybrid Retrieval: kein belegter Treffer.';
                return;
            }
            for (const item of data.evidence) {
                if (!item?.source?.title || !item?.source?.path || typeof item.text !== 'string') continue;
                const article = document.createElement('article');
                article.className = 'brain-retrieval-result';
                const title = document.createElement('strong');
                title.textContent = item.source.title;
                const meta = document.createElement('small');
                meta.textContent = item.source.path + (item.observed_at ? ' · Stand ' + item.observed_at : '');
                const body = document.createElement('p');
                body.textContent = item.text;
                article.append(title, meta, body);
                ui.retrievalResults.append(article);
            }
            ui.status.textContent = data.evidence.length + ' belegte Passage' + (data.evidence.length === 1 ? '' : 'n') + ' aus dem Hybrid Retrieval.';
        } catch (error) {
            ui.retrievalResults.replaceChildren();
            const failed = document.createElement('p');
            failed.textContent = error.name === 'AbortError'
                ? 'Die hybride Wissenssuche hat ihr Zeitbudget überschritten.'
                : error.message;
            ui.retrievalResults.append(failed);
            ui.status.textContent = 'Hybrid Retrieval nicht verfügbar.';
        } finally {
            clearTimeout(timer);
            ui.semanticSearch.disabled = false;
        }
    }

    function draw() { renderMap(); }

    function renderMap() {
        if (!graph || !window.vis?.Network) return;
        let entries = [];
        let edges = [];
        const sourceGroups = new Map();
        const expectedFilter = defaultFilter();
        const searching = Boolean(ui.search.value.trim()) || ui.filter.value !== expectedFilter;
        const overviewMode = view.type === 'overview' && !searching;
        const knowledgeMode = mapMode === 'knowledge';
        byId('brain-map-fit').textContent = window.innerWidth <= 720 ? 'Lesbare Größe' : 'Einpassen';
        ui.layout.classList.toggle('brain-overview', view.type !== 'node' || searching);
        ui.layout.classList.toggle('brain-results-open', searching || view.type === 'topic');
        byId('brain-map-detail').hidden = view.type !== 'node' || searching;

        if (overviewMode) {
            const visibleTopics = [...topics].filter(([key]) => !knowledgeMode || key.startsWith('docs:'));
            entries = visibleTopics.map(([id, members]) => ({
                id,
                label: topicName(id),
                kind: id.startsWith('docs:') ? 'document' : 'code',
                member_count: members.length,
                subtitle: members.length + (id.startsWith('docs:')
                    ? (members.length === 1 ? ' Dokument' : ' Dokumente')
                    : ' technische Knoten')
            }));
            if (!knowledgeMode) {
                const aggregate = new Map();
                for (const edge of graph.links) {
                    const source = nodes.get(edge.source), target = nodes.get(edge.target);
                    if (!source || !target) continue;
                    const from = topicKey(source), to = topicKey(target);
                    if (from === to) continue;
                    const key = JSON.stringify([from, to]);
                    const current = aggregate.get(key) || {from, to, count: 0};
                    current.count++;
                    aggregate.set(key, current);
                }
                edges = [...aggregate.values()].map(edge => ({
                    ...edge,
                    label: String(edge.count),
                    width: Math.min(4, 1 + Math.log2(edge.count + 1))
                }));
            }
            byId('brain-map-view-title').textContent = knowledgeMode ? 'Wissenslandschaft' : 'Technik & Belege';
            ui.status.textContent = knowledgeMode
                ? 'Wähle ein Wissensgebiet oder suche direkt nach einem Thema.'
                : 'Wähle einen Bereich, um technische Belege und Abhängigkeiten zu erkunden.';
            ui.scope.textContent = knowledgeMode
                ? 'Die Startansicht bündelt Wissensdokumente nach Thema. Technische Knoten erscheinen erst in der Technikansicht.'
                : 'Die Technikansicht zeigt Dokumente, Codebereiche und die vorhandenen Belegbeziehungen.';
        } else {
            let ids;
            if (view.type === 'node' && !searching) {
                if (knowledgeMode) {
                    ids = knowledgeNeighborhood(selected);
                } else {
                    const incident = neighbors.get(selected) || [];
                    ids = new Set([selected, ...incident.flatMap(edge => [edge.source, edge.target])]);
                }
                byId('brain-map-view-title').textContent = 'Verbindungen · ' + (nodes.get(selected)?.label || 'Auswahl');
            } else {
                ids = new Set(graph.nodes.filter(matches).map(node => node.id));
                byId('brain-map-view-title').textContent = searching ? 'Suchergebnisse' : topicName(view.key);
            }
            entries = [...ids]
                .map(id => nodes.get(id))
                .filter(Boolean)
                .map(node => ({
                    ...node,
                    subtitle: isKnowledgeNode(node)
                        ? (node.visibility === 'public' || node.source_file.startsWith('public/')
                            ? 'Öffentliche Wissensquelle'
                            : 'Interne Dokumentation')
                        : node.source_file.split('/').pop() + (node.source_location ? ':' + node.source_location : '')
                }));
            if (knowledgeMode) {
                edges = sharedKnowledgeEdges(ids).map(edge => ({
                    from: edge.source,
                    to: edge.target,
                    label: edge.count > 1 ? 'gemeinsame Belege · ' + edge.count : 'gemeinsamer Beleg',
                    dashes: true,
                    relation: edge.relation,
                    color: '#697185'
                }));
                ui.scope.textContent = entries.length + ' Wissensknoten · ' + edges.length + ' abgeleitete Beziehungen. Eine gestrichelte Linie bedeutet, dass Dokumente mindestens denselben technischen Beleg referenzieren.';
            } else {
                edges = graph.links
                    .filter(edge => ids.has(edge.source) && ids.has(edge.target))
                    .map(edge => ({
                        from: edge.source,
                        to: edge.target,
                        label: relationLabels[edge.relation] || edge.relation,
                        dashes: edge.confidence === 'INFERRED',
                        relation: edge.relation,
                        color: edge.relation === 'documents' ? '#a38a55' : '#5c6372'
                    }));
                ui.scope.textContent = entries.length + ' Knoten · ' + edges.length + ' Beziehungen. Gestrichelte Linien sind abgeleitet.';
            }
        }

        if (!knowledgeMode && view.type === 'node' && !searching) {
            const originals = entries;
            const mapped = new Map();
            const expanded = new Set(view.expanded || []);
            const individual = [];
            for (const entry of originals) {
                if (entry.id === selected || expanded.has(entry.source_file)) {
                    mapped.set(entry.id, entry.id);
                    individual.push(entry);
                    continue;
                }
                const key = '@source:' + entry.source_file;
                mapped.set(entry.id, key);
                if (!sourceGroups.has(key)) {
                    sourceGroups.set(key, {
                        id: key,
                        label: entry.source_file.split('/').pop() || entry.source_file,
                        source_file: entry.source_file,
                        kind: entry.kind,
                        members: [],
                        internal: 0
                    });
                }
                sourceGroups.get(key).members.push(entry.id);
            }
            const groupedEdges = new Map();
            for (const edge of edges) {
                const from = mapped.get(edge.from), to = mapped.get(edge.to);
                if (from === to && sourceGroups.has(from)) {
                    sourceGroups.get(from).internal++;
                    continue;
                }
                const key = JSON.stringify([from, to, edge.label, Boolean(edge.dashes)]);
                const existing = groupedEdges.get(key) || {...edge, from, to, count: 0};
                existing.count++;
                groupedEdges.set(key, existing);
            }
            edges = [...groupedEdges.values()].map(edge => ({
                ...edge,
                label: edge.label + (edge.count > 1 ? ' · ' + edge.count : '')
            }));
            entries = [
                ...individual,
                ...[...sourceGroups.values()].map(group => ({
                    ...group,
                    member_count: group.members.length,
                    subtitle: group.members.length + ' Knoten · ' + group.internal + ' interne Beziehungen'
                }))
            ];
            ui.scope.textContent = originals.length + ' Knoten in dieser Nachbarschaft. Knoten derselben Quelldatei sind gebündelt und lassen sich aufklappen.';
        }

        entries.sort((a, b) =>
            (view.type === 'node' && !searching ? Number(b.id === selected) - Number(a.id === selected) : 0)
            || (isKnowledgeNode(a) ? 0 : 1) - (isKnowledgeNode(b) ? 0 : 1)
            || a.label.localeCompare(b.label, 'de')
        );

        const columns = ui.canvas.clientWidth < 600 ? 1 : overviewMode ? 4 : 3;
        const compactFocus = view.type === 'node' && !searching && entries.length <= 6;
        const usePhysics = entries.length > 1 && entries.length <= 240
            && (overviewMode || knowledgeMode || view.type === 'topic');
        function position(entry, index) {
            if (compactFocus && columns > 1) {
                if (entry.id === selected) return {x: 0, y: 0};
                return {x: ((index - 1) % columns - 1) * 300, y: 210 + Math.floor((index - 1) / columns) * 170};
            }
            return {
                x: (index % columns - (columns - 1) / 2) * (overviewMode ? 240 : 300),
                y: Math.floor(index / columns) * (overviewMode ? 120 : 165)
            };
        }

        const data = {
            nodes: entries.map((entry, index) => {
                const knowledge = isKnowledgeNode(entry);
                const cluster = overviewMode || entry.id.startsWith('docs:') || entry.id.startsWith('code:');
                const count = Number(entry.member_count || 1);
                const size = cluster ? Math.min(38, 16 + Math.sqrt(count) * 3.4) : (entry.id === selected ? 21 : 15);
                const label = wrap(entry.label, cluster ? 24 : 30) + (cluster ? '\n' + count : '');
                return {
                    id: entry.id,
                    label,
                    title: entry.subtitle || '',
                    shape: 'dot',
                    size,
                    ...(usePhysics ? {} : position(entry, index)),
                    color: {
                        background: knowledge ? '#b79b62' : '#596071',
                        border: entry.id === selected ? '#f4e5bd' : knowledge ? '#e0c47f' : '#81899b',
                        highlight: {background: knowledge ? '#d5b66d' : '#727b90', border: '#fff1ca'},
                        hover: {background: knowledge ? '#c8a95f' : '#697287', border: '#e8d9b0'}
                    },
                    borderWidth: entry.id === selected ? 3 : 1.5,
                    font: {
                        color: '#e9ebf0',
                        size: cluster ? 15 : 13,
                        face: 'system-ui',
                        vadjust: -6,
                        strokeWidth: 5,
                        strokeColor: '#090b10'
                    },
                    shadow: {enabled: true, color: '#00000080', size: 10, x: 0, y: 4}
                };
            }),
            edges: edges.map((edge, index) => ({
                ...edge,
                id: index,
                arrows: knowledgeMode ? undefined : {to: {enabled: true, scaleFactor: .45}},
                width: edge.width || 1,
                font: {
                    size: 10,
                    color: '#b9bec9',
                    strokeWidth: 4,
                    strokeColor: '#090b10',
                    align: 'horizontal'
                },
                smooth: {enabled: true, type: 'continuous', roundness: .16}
            }))
        };

        network?.destroy();
        network = new window.vis.Network(ui.canvas, data, {
            autoResize: true,
            interaction: {hover: true, keyboard: false, zoomView: true, dragView: true},
            layout: {improvedLayout: false},
            physics: usePhysics ? {
                enabled: true,
                solver: 'forceAtlas2Based',
                stabilization: {enabled: true, iterations: 120, updateInterval: 20, fit: true},
                forceAtlas2Based: {
                    gravitationalConstant: -48,
                    centralGravity: .008,
                    springLength: overviewMode ? 190 : 145,
                    springConstant: .045,
                    damping: .52,
                    avoidOverlap: .35
                }
            } : false
        });
        if (usePhysics) {
            network.once('stabilizationIterationsDone', () => {
                network?.setOptions({physics: false});
                network?.fit({animation: {duration: 220, easingFunction: 'easeInOutQuad'}});
            });
        }
        network.on('click', params => {
            if (!params.nodes.length) return;
            const id = params.nodes[0];
            if (overviewMode) {
                openTopic(id);
            } else if (sourceGroups.has(id)) {
                view.expanded = [...(view.expanded || []), sourceGroups.get(id).source_file];
                renderMap();
            } else {
                view = {type: 'node'};
                ui.search.value = '';
                ui.filter.value = defaultFilter();
                select(id);
                showResults();
            }
        });
        if (!usePhysics) {
            if ((overviewMode || entries.length <= 6) && ui.canvas.clientWidth > 600) {
                network.fit({animation: false});
            } else {
                const scale = ui.canvas.clientWidth < 600
                    ? Math.max(.7, Math.min(1, ui.canvas.clientWidth / 280))
                    : .85;
                network.moveTo({
                    position: {x: 0, y: ui.canvas.clientHeight / (2 * scale) - 90},
                    scale,
                    animation: false
                });
            }
        }

        const picker = document.querySelector('.brain-map-topic-picker');
        if (window.innerWidth <= 720) picker.open = false;
        const nav = byId('brain-map-topics');
        nav.replaceChildren();
        if (!knowledgeMode) {
            for (const group of sourceGroups.values()) {
                const button = document.createElement('button');
                button.type = 'button';
                button.textContent = 'Aufklappen: ' + group.label + ' · ' + group.members.length;
                button.addEventListener('click', () => {
                    view.expanded = [...(view.expanded || []), group.source_file];
                    renderMap();
                });
                nav.append(button);
            }
        }
        const orderedTopics = [...topics]
            .filter(([key]) => !knowledgeMode || key.startsWith('docs:'))
            .sort(([a], [b]) =>
                Number(b.startsWith('docs:')) - Number(a.startsWith('docs:'))
                || topicName(a).localeCompare(topicName(b), 'de')
            );
        for (const [key, members] of orderedTopics) {
            const button = document.createElement('button');
            button.type = 'button';
            button.textContent = topicName(key) + ' · ' + members.length;
            button.setAttribute('aria-pressed', String(view.type === 'topic' && view.key === key));
            button.addEventListener('click', () => {
                ui.search.value = '';
                ui.filter.value = defaultFilter();
                openTopic(key);
            });
            nav.append(button);
        }
    }

    function select(id) {
        const node = nodes.get(id);
        if (!node) return;
        selected = id;
        for (const button of ui.results.querySelectorAll('button[data-node-id]')) {
            button.setAttribute('aria-pressed', String(button.dataset.nodeId === id));
        }
        ui.title.textContent = node.label || node.id;
        const knowledge = isKnowledgeNode(node);
        ui.meta.textContent = [
            knowledge
                ? (node.visibility === 'public' || node.source_file.startsWith('public/')
                    ? 'Öffentliche Wissensquelle'
                    : 'Interne Dokumentation')
                : 'Technischer Beleg',
            node.group,
            node.stand ? 'Stand: ' + node.stand : ''
        ].filter(Boolean).join(' · ');
        ui.source.value = [node.source_file, node.source_location].filter(Boolean).join(':');

        const rawIncident = neighbors.get(id) || [];
        let incident = rawIncident;
        if (mapMode === 'knowledge' && knowledge) {
            incident = sharedKnowledgeEdges(knowledgeNeighborhood(id))
                .filter(link => link.source === id || link.target === id);
            const evidenceCount = rawIncident.filter(link => link.relation === 'documents' && link.source === id).length;
            if (incident.length) {
                ui.evidence.textContent = incident.length + (incident.length === 1 ? ' Wissensdokument teilt' : ' Wissensdokumente teilen')
                    + ' mindestens einen technischen Beleg mit dieser Quelle. '
                    + evidenceCount + ' direkte technische Belege sind in der Technikansicht nachvollziehbar.';
            } else if (evidenceCount) {
                ui.evidence.textContent = evidenceCount + ' technische Belege sind vorhanden. In diesem Ausschnitt ergibt sich daraus noch keine Verbindung zu einem weiteren Wissensdokument.';
            } else {
                ui.evidence.textContent = 'Für dieses Dokument ist in der aktuellen Karte noch kein technischer Beleg hinterlegt.';
            }
        } else if (knowledge) {
            const documents = rawIncident.filter(link => link.relation === 'documents' && link.source === id);
            ui.evidence.textContent = documents.length
                ? documents.length + ' direkte technische Belege sind mit diesem Dokument verknüpft.'
                : 'Für dieses Dokument ist in der aktuellen Karte noch kein technischer Beleg hinterlegt.';
        } else {
            ui.evidence.textContent = 'Dieser Knoten gehört zur technischen Belegebene. Abgeleitete Beziehungen sind gestrichelt dargestellt.';
        }

        ui.links.replaceChildren();
        const heading = document.createElement('h5');
        heading.textContent = mapMode === 'knowledge'
            ? 'Verwandtes Wissen (' + incident.length + ')'
            : 'Verbindungen (' + incident.length + ')';
        ui.links.append(heading);
        for (const link of incident) {
            const target = nodes.get(link.source === id ? link.target : link.source);
            if (!target || (mapMode === 'knowledge' && !isKnowledgeNode(target))) continue;
            const button = document.createElement('button');
            button.type = 'button';
            if (link.relation === 'shared_evidence') {
                button.textContent = '↔ Gemeinsamer technischer Beleg: ' + (target.label || target.id);
            } else {
                const direction = link.source === id ? '→' : '←';
                button.textContent = direction + ' ' + (relationLabels[link.relation] || link.relation || 'Verbindung') + ': '
                    + (target.label || target.id) + (link.confidence === 'INFERRED' ? ' (abgeleitet)' : '');
            }
            button.addEventListener('click', () => {
                view = {type: 'node'};
                ui.search.value = '';
                ui.filter.value = defaultFilter();
                select(target.id);
                showResults();
                ui.title.focus?.();
            });
            ui.links.append(button);
        }
        if (!incident.length) {
            const empty = document.createElement('p');
            empty.textContent = mapMode === 'knowledge'
                ? 'In der aktuellen Wissenslandschaft sind noch keine verwandten Dokumente über gemeinsame Belege verknüpft.'
                : 'In dieser Karte sind noch keine Verbindungen für den Knoten hinterlegt.';
            ui.links.append(empty);
        }
        ui.status.textContent = 'Ausgewählt: ' + (node.label || node.id) + '.';
        draw();
    }

    function renderGameStatus(snapshot) {
        const summary=byId('brain-game-summary'), sources=byId('brain-game-sources');
        sources.replaceChildren();
        if(!snapshot) { summary.textContent='Kein bestätigter Spielquellenstand verfügbar. Community- und Spielwissen haben getrennte Quellenstände.'; return; }
        const date=value=>value?new Date(value).toLocaleString('de-DE'):'nicht belegt';
        summary.textContent=snapshot.entries+' Wissenseinträge. Darstellung erstellt: '+date(snapshot.rendered_at)+'. Das ist kein neuer Abruf der Spielquellen und keine Bestätigung des heutigen Spielstands.';
        for(const [name,source] of Object.entries(snapshot.provenance)) {
            const item=document.createElement('p');
            const revisions=Object.entries(source.revisions).map(([sha,at])=>date(at)+' · Revision '+sha.slice(0,12)).join('; ') || 'Quellrevision nicht belegt';
            const label=name==='deadlock_data'?'Spieldaten':name==='deadlock_wiki'?'Spiel-Wiki':name;
            item.textContent=label+': '+source.entries+' Einträge. Quellstand: '+revisions+'. Zuletzt abgerufen: '+date(source.latest_fetched_at)+'.';sources.append(item);
        }
    }

    async function loadKnowledgeStatus() {
        const generation=++statusGeneration;
        statusController?.abort();
        const summary=byId('brain-knowledge-summary'), stamp=byId('brain-knowledge-stamp');
        const repositories=byId('brain-knowledge-repositories'), gaps=byId('brain-knowledge-gaps');
        summary.textContent='Prüfstand wird geladen …';stamp.textContent='';repositories.replaceChildren();gaps.replaceChildren();
        byId('brain-game-summary').textContent='Quellenstand wird geladen …';byId('brain-game-sources').replaceChildren();
        const controller=new AbortController(), timer=setTimeout(()=>controller.abort(),10000);
        statusController=controller;
        try {
            const response=await fetch('/api/brain/knowledge-status',{credentials:'same-origin',signal:controller.signal});
            if (!response.ok) throw new Error('unavailable');
            const status=await response.json();
            if(generation!==statusGeneration) return;
            if(status.schema_version!==1 || !status.totals || !Array.isArray(status.repositories) || !Array.isArray(status.gaps)) throw new Error('invalid');
            const totals=status.totals;
            renderGameStatus(status.game_snapshot);
            summary.textContent=totals.public_documents+' öffentliche Dokumente · '+totals.verified_documents+' sachlich geprüft · '+totals.pending_review+' noch zu prüfen · '+totals.excluded_documents+' ausgeschlossen.';
            const generated=new Date(status.generated_at);
            stamp.textContent='Statuslauf: '+generated.toLocaleString('de-DE')+'. Zuletzt bestätigter Indexstand: '+(status.active_snapshot || 'nicht belegt')+'. Der Statuslauf ist keine neue fachliche Prüfung.'+(status.refresh_status==='failed'?' Der letzte Aktualisierungslauf ist fehlgeschlagen; die Angaben beziehen sich auf den vorhandenen Stand.':'');
            for(const repo of status.repositories) {
                const section=document.createElement('section');
                const title=document.createElement('h5');title.textContent=repo.label;
                const text=document.createElement('p');
                text.textContent=repo.public_documents+' öffentlich · '+repo.verified_documents+' geprüft · '+repo.pending_review+' offen. Quellenrevision: '+repo.revision+'. Letzte belegte Fachprüfung: '+(repo.last_verified_at ? new Date(repo.last_verified_at).toLocaleString('de-DE') : 'noch keine')+'.';
                section.append(title,text);repositories.append(section);
            }
            for(const gap of status.gaps) {
                const item=document.createElement('li');
                item.textContent=gap.title+': '+gap.reason+' ('+(gap.status==='excluded'?'ausgeschlossen':'noch zu prüfen')+').';gaps.append(item);
            }
            if(!status.gaps.length) { const item=document.createElement('li');item.textContent='In diesem Statuslauf sind keine einzelnen Lücken aufgeführt. Das ist keine Zusage vollständigen Wissens.';gaps.append(item); }
        } catch (_) { if(generation!==statusGeneration) return; renderGameStatus(null);repositories.replaceChildren();gaps.replaceChildren();summary.textContent='Noch kein belegter Wissensprüfstand verfügbar.';stamp.textContent='Die Wissenskarte darunter bleibt eine separate Systemübersicht. Daraus lässt sich kein aktueller fachlicher Prüfstand ableiten.'; }
        finally {clearTimeout(timer);}
    }

    async function load(force = false) {
        if (loading || (graph && !force)) return;
        loading = true;
        void loadKnowledgeStatus();
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
            topics = new Map();
            for (const node of data.nodes) { const key = topicKey(node); if (!topics.has(key)) topics.set(key, []); topics.get(key).push(node); }
            nodes = new Map(data.nodes.map(node => [node.id, node]));
            neighbors = new Map(data.nodes.map(node => [node.id, []]));
            for (const link of data.links) {
                if (!nodes.has(link.source) || !nodes.has(link.target)) continue;
                neighbors.get(link.source).push(link);
                if (link.source !== link.target) neighbors.get(link.target).push(link);
            }
            const stamp = new Date(data.generated_at).toLocaleString('de-DE');
            ui.summary.textContent = data.corpus_nodes + ' kartierte Dokumente · ' + data.nodes.length + ' Knoten · ' + data.links.length + ' Verbindungen. Ausschnitt: Wissensbasis, belegte Code-Bezüge und deren direkte Nachbarn. ' + data.unlinked_documents + ' Dokumente haben in der Ausgangskarte noch keine Code-Belege. Erzeugt: ' + stamp + '. Das Erzeugungsdatum bestätigt keine sachliche Prüfung der Inhalte.' + (data.omitted_nodes || data.omitted_links ? ' Die Größe ist begrenzt; ' + data.omitted_nodes + ' weitere Nachbarn und ' + data.omitted_links + ' Verbindungen sind ausgeblendet.' : '');
            ui.layout.hidden = false;
            showResults();
            const initial = data.nodes.find(node => node.id === selected) || data.nodes.find(node => node.kind === 'document' && /concierge/.test(node.source_file) && (neighbors.get(node.id)?.length || 0) > 0) || data.nodes.find(node => node.kind === 'document' && (neighbors.get(node.id)?.length || 0) > 0) || data.nodes.find(node => node.kind === 'document') || data.nodes[0];
            if (view.type === 'node') select(initial.id); else selected = null;
            showResults();
            try { await loadLibrary(); const current = nodes.get(selected) || initial; draw(current, neighbors.get(current.id) || []); }
            catch (error) { ui.error.textContent = error.message + ' Suche und Verbindungsliste bleiben bedienbar.'; ui.error.hidden = false; }
        } catch (error) {
            graph = null;
            ui.summary.textContent = 'Kein aktueller Kartenstand verfügbar.';
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

    ui.search.addEventListener('input', () => {
        view = {type: 'overview'};
        pageSize = 60;
        ui.retrievalResults.hidden = true;
        ui.retrievalResults.replaceChildren();
        showResults();
        renderMap();
    });
    ui.search.addEventListener('keydown', event => {
        if (event.key === 'Enter') {
            event.preventDefault();
            void semanticSearch();
        }
    });
    ui.filter.addEventListener('change', () => { view = {type: 'overview'}; pageSize = 60; showResults(); renderMap(); });
    ui.semanticSearch.addEventListener('click', () => void semanticSearch());
    ui.more.addEventListener('click', () => { const previous = pageSize; pageSize += 60; showResults(); ui.results.querySelectorAll('button')[previous]?.focus(); });
    byId('brain-map-reload').addEventListener('click', () => load(true));
    byId('brain-map-copy').addEventListener('click', async () => {
        try { await navigator.clipboard.writeText(ui.source.value); ui.status.textContent = 'Quellenpfad kopiert.'; }
        catch (_) { ui.source.focus(); ui.source.select(); ui.status.textContent = 'Quellenpfad markiert. Kopiere ihn mit Strg+C oder ⌘C.'; }
    });
    byId('brain-map-overview').addEventListener('click', overview);
    ui.knowledgeMode.addEventListener('click', () => setMapMode('knowledge'));
    ui.systemMode.addEventListener('click', () => setMapMode('system'));
    ui.detailClose.addEventListener('click', overview);
    byId('brain-map-fit').addEventListener('click', () => { if(window.innerWidth<=720) renderMap(); else network?.fit({animation:false}); });
    byId('brain-map-zoom-in').addEventListener('click', () => network?.moveTo({scale:network.getScale()*1.3,animation:false}));
    byId('brain-map-zoom-out').addEventListener('click', () => network?.moveTo({scale:network.getScale()/1.3,animation:false}));
    document.addEventListener('keydown', event => {
        const typing = event.target instanceof HTMLElement && event.target.matches('input, textarea, select, [contenteditable="true"]');
        if (event.key === '/' && !typing && document.querySelector('.tab-panel[data-tab="brain"].active')) {
            event.preventDefault();
            ui.search.focus();
            ui.search.select();
        } else if (event.key === 'Escape' && document.querySelector('.tab-panel[data-tab="brain"].active')) {
            if (view.type === 'node' || ui.search.value) {
                event.preventDefault();
                overview();
                ui.search.focus();
            }
        }
    });
    let resizeTimer;
    window.addEventListener('resize', () => { clearTimeout(resizeTimer); resizeTimer=setTimeout(renderMap,120); });
    setMapMode('knowledge');
    window.loadVisualBrain = load;
    if (document.querySelector('.tab-panel[data-tab="brain"].active')) load();
})();
