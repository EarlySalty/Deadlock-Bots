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
        const relationLabels = {documents: 'belegt', calls: 'ruft auf', contains: 'enthält', defines: 'definiert', imports: 'importiert', imports_from: 'importiert aus', uses: 'nutzt', references: 'verweist auf', depends_on: 'hängt ab von', rationale_for: 'begründet', method: 'Methode'};
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

    function wrap(value, width = 28) {
        return String(value).match(new RegExp('.{1,' + width + '}(?:\\s|$)|.{1,' + width + '}', 'g'))?.map(line => line.trim()).join('\n') || value;
    }

    function openTopic(key) {
        view = {type: 'topic', key};
        pageSize = 60;
        showResults();
        renderMap();
        ui.status.textContent = topicName(key) + ' geöffnet. Wähle ein Dokument oder einen Code-Knoten.';
    }

    function overview() {
        selected = null;
        view = {type: 'overview'};
        ui.search.value = '';
        ui.filter.value = 'all';
        showResults();
        renderMap();
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
        if (query && !text.includes(query)) return false;
        if (view.type === 'topic' && topicKey(node) !== view.key) return false;
        if (view.type === 'node' && node.id !== selected && !(neighbors.get(selected) || []).some(edge => edge.source === node.id || edge.target === node.id)) return false;
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
            button.addEventListener('click', () => { view = {type: 'node'}; ui.search.value = ''; ui.filter.value = 'all'; select(node.id); showResults(); });
            ui.results.append(button);
        }
        if (!matchesList.length) ui.results.textContent = 'Keine Treffer. Versuche einen anderen Begriff oder Bereich.';
        ui.more.hidden = matchesList.length <= pageSize;
        ui.more.textContent = 'Weitere Treffer (' + Math.max(0, matchesList.length - pageSize) + ')';
    }

    function draw() { renderMap(); }

    function renderMap() {
        if (!graph || !window.vis?.Network) return;
        let entries = [];
        let edges = [];
        const sourceGroups = new Map();
        byId('brain-map-fit').textContent=window.innerWidth<=720?'Lesbare Größe':'Alles einpassen';
        const searching = ui.search.value.trim() || ui.filter.value !== 'all';
        const overviewMode = view.type === 'overview' && !searching;
        ui.layout.classList.toggle('brain-overview', view.type !== 'node' || Boolean(searching));
        byId('brain-map-detail').hidden = view.type !== 'node' || Boolean(searching);
        if (overviewMode) {
            ui.status.textContent = 'Wähle ein Thema, um Dokumente und Beziehungen zu erkunden.';
            entries = [...topics].map(([id, members]) => ({id, label: topicName(id), kind: id.startsWith('docs:') ? 'document' : 'code', subtitle: members.length + (id.startsWith('docs:') ? (members.length === 1 ? ' Dokument' : ' Dokumente') : ' Systemknoten')}));
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
            edges = [...aggregate.values()].map(edge => ({...edge, label: String(edge.count), width: 1.5}));
            byId('brain-map-view-title').textContent = 'Themen & Quellbereiche';
            ui.scope.textContent = (window.innerWidth<=720?'Alle Bereiche stehen in der Themenliste oben. Nach unten scrollen zeigt weitere Bereiche. ':'') + 'Jedes Feld bündelt Dokumente oder Code desselben Moduls. Pfeile zählen vorhandene Beziehungen zwischen Bereichen. Ein Klick öffnet den Bereich. Themen ohne Linien haben noch keine hinterlegten Verknüpfungen. Ziehen verschiebt die Karte; +/− zoomt.';
        } else {
            let ids;
            if (view.type === 'node' && !searching) {
                const incident = neighbors.get(selected) || [];
                ids = new Set([selected, ...incident.flatMap(edge => [edge.source, edge.target])]);
                byId('brain-map-view-title').textContent = 'Verbindungen · ' + (nodes.get(selected)?.label || 'Auswahl');
            } else {
                ids = new Set(graph.nodes.filter(matches).map(node => node.id));
                byId('brain-map-view-title').textContent = searching ? 'Suchergebnisse in der Karte' : topicName(view.key);
            }
            entries = [...ids].map(id => nodes.get(id)).filter(Boolean).map(node => ({...node, subtitle: node.kind === 'document' ? (node.source_file.startsWith('public/') ? 'Öffentliche Dokumentation' : 'Interne Dokumentation') : node.source_file.split('/').pop() + (node.source_location ? ':' + node.source_location : '')}));
            // All edges between displayed nodes, not only the selected node's spokes.
            edges = graph.links.filter(edge => ids.has(edge.source) && ids.has(edge.target)).map(edge => ({from: edge.source, to: edge.target, label: relationLabels[edge.relation] || edge.relation, dashes: edge.confidence === 'INFERRED', color: edge.relation === 'documents' ? '#d6a62c' : '#755d3c'}));
            ui.scope.textContent = entries.length + ' Knoten · ' + edges.length + ' Beziehungen, ohne zusätzliche Anzeigegrenze. Alle Namen stehen im Feld und in der Trefferliste. Ziehen verschiebt die Karte, +/− zoomt; „Alles einpassen“ zeigt den gesamten Ausschnitt. Gestrichelte Beziehungen sind abgeleitet.';
        }
        if (view.type === 'node' && !searching) {
            const originals = entries;
            const mapped = new Map();
            const expanded = new Set(view.expanded || []);
            const individual = [];
            for (const entry of originals) {
                if (entry.id === selected || expanded.has(entry.source_file)) { mapped.set(entry.id,entry.id); individual.push(entry); continue; }
                const key = '@source:' + entry.source_file;
                mapped.set(entry.id,key);
                if (!sourceGroups.has(key)) sourceGroups.set(key,{id:key,label:entry.source_file.split('/').pop() || entry.source_file,source_file:entry.source_file,kind:entry.kind,members:[],internal:0});
                sourceGroups.get(key).members.push(entry.id);
            }
            const groupedEdges = new Map();
            for (const edge of edges) {
                const from=mapped.get(edge.from),to=mapped.get(edge.to);
                if (from === to && sourceGroups.has(from)) { sourceGroups.get(from).internal++; continue; }
                const key=JSON.stringify([from,to,edge.label,Boolean(edge.dashes)]);
                const existing=groupedEdges.get(key)||{...edge,from,to,count:0};existing.count++;groupedEdges.set(key,existing);
            }
            edges=[...groupedEdges.values()].map(edge=>({...edge,label:edge.label+(edge.count>1?' · '+edge.count:'')}));
            entries=[...individual,...[...sourceGroups.values()].map(group=>({...group,subtitle:group.members.length+' Knoten · '+group.internal+' interne Beziehungen\nZum Aufklappen auswählen'}))];
            ui.scope.textContent = originals.length+' Knoten in dieser Nachbarschaft. Knoten derselben Quelldatei sind gebündelt; Pfeile zählen echte Beziehungen. Öffne ein Bündel, um alle Einzelknoten zu sehen. Es wird kein Nachbar weggelassen. Mit einem Finger die Karte verschieben; alle Ziele stehen auch in der Verbindungsliste darunter. Gestrichelt: abgeleitete Beziehung.';
        }
        entries.sort((a,b) => (view.type === 'node' && !searching ? Number(b.id === selected) - Number(a.id === selected) : 0) || (a.kind === 'document' ? 0 : 1) - (b.kind === 'document' ? 0 : 1) || a.label.localeCompare(b.label, 'de'));
        const columns = ui.canvas.clientWidth < 600 ? 1 : overviewMode ? 4 : 3;
        const compactFocus = view.type === 'node' && !searching && entries.length <= 6;
        ui.canvas.style.height = compactFocus ? '460px' : '';
        function position(entry,index) {
            if (compactFocus && columns > 1) {
                if (entry.id === selected) return {x:0,y:0};
                return {x:((index-1)%columns-1)*340,y:240+Math.floor((index-1)/columns)*190};
            }
            return {x:(index%columns-(columns-1)/2)*(overviewMode ? 275 : 340),y:Math.floor(index/columns)*(overviewMode ? 125 : 175)};
        }
        const data = {
            nodes: entries.map((entry, index) => ({id: entry.id, label: wrap(entry.label) + '\n\n' + wrap(entry.subtitle, 34), shape: 'box', margin: 12, widthConstraint: {minimum: overviewMode ? 195 : 210, maximum: overviewMode ? 220 : 240}, heightConstraint: {minimum: overviewMode ? 50 : 65},
                ...position(entry,index),
                color: {background: entry.kind === 'document' ? '#151311' : '#0e0d0c', border: entry.id === selected ? '#fff0be' : entry.kind === 'document' ? '#d0a349' : '#765c39', highlight: {background: '#25221c', border: '#ffe3a2'}, hover: {background: '#201e1a', border: '#f0cb76'}},
                borderWidth: entry.id === selected ? 3 : 1.5, font: {color: '#fff3db', size: overviewMode ? 19 : 15, face: 'system-ui', align: 'center'}, shadow: {enabled: true, color: '#000000', size: 12, x: 0, y: 5}})),
            edges: edges.map((edge,index) => ({...edge,id:index,arrows:{to:{enabled:true,scaleFactor:.55}},font:{size:11,color:'#ead6a8',strokeWidth:4,strokeColor:'#0d0806',align:'horizontal'},color:edge.color || '#a18044',smooth:{enabled:true,type:'continuous',roundness:.12}})),
        };
        network?.destroy();
        network = new window.vis.Network(ui.canvas, data, {autoResize:true,physics:false,interaction:{hover:true,keyboard:false,zoomView:true,dragView:true},layout:{improvedLayout:false}});
        network.on('click', params => { if (!params.nodes.length) return; const id=params.nodes[0]; if (overviewMode) openTopic(id); else if (sourceGroups.has(id)) { view.expanded = [...(view.expanded || []), sourceGroups.get(id).source_file]; renderMap(); } else { view={type:'node'}; ui.search.value=''; ui.filter.value='all'; select(id); showResults(); } });
        if ((overviewMode || entries.length <= 6) && ui.canvas.clientWidth > 600) network.fit({animation:false});
        else { const scale=ui.canvas.clientWidth<600?Math.max(.7,Math.min(1,ui.canvas.clientWidth/280)):.85; network.moveTo({position:{x:0,y:ui.canvas.clientHeight/(2*scale)-90},scale,animation:false}); }
        const picker=document.querySelector('.brain-map-topic-picker');
        if (window.innerWidth <= 720) picker.open=true;
        const nav = byId('brain-map-topics');
        nav.replaceChildren();
        for (const group of sourceGroups.values()) {
            const button=document.createElement('button');button.type='button';button.textContent='Aufklappen: '+group.label+' · '+group.members.length;
            button.addEventListener('click',()=>{view.expanded=[...(view.expanded||[]),group.source_file];renderMap();});nav.append(button);
        }
        const orderedTopics=[...topics].sort(([a],[b])=>Number(b.startsWith('docs:'))-Number(a.startsWith('docs:'))||topicName(a).localeCompare(topicName(b),'de'));
        for (const [key,members] of orderedTopics) {
            const button=document.createElement('button');button.type='button';button.textContent=topicName(key)+' · '+members.length;
            button.setAttribute('aria-pressed',String(view.type==='topic'&&view.key===key));
            button.addEventListener('click',()=>{ui.search.value='';ui.filter.value='all';openTopic(key);});nav.append(button);
        }
    }

    function select(id) {
        const node = nodes.get(id);
        if (!node) return;
        selected = id;
        for (const button of ui.results.querySelectorAll('button[data-node-id]')) button.setAttribute('aria-pressed', String(button.dataset.nodeId === id));
        ui.title.textContent = node.label || node.id;
        ui.meta.textContent = [node.kind === 'document' ? (node.source_file.startsWith('public/') ? 'Öffentliche Wissensquelle' : 'Interne Dokumentation · kein Concierge-Antwortwissen') : 'Code', node.group, node.stand ? 'Dokumentangabe: ' + node.stand : ''].filter(Boolean).join(' · ');
        ui.source.value = [node.source_file, node.source_location].filter(Boolean).join(':');
        const incident = neighbors.get(id) || [];
        const documents = incident.filter(link => link.relation === 'documents' && link.source === id);
        ui.evidence.textContent = node.kind === 'document'
            ? documents.length ? documents.length + ' belegte Code-Verbindungen in diesem Ausschnitt.' + (node.evidence_links > documents.length ? ' Weitere Belege liegen außerhalb der begrenzten Ansicht.' : ' Die Linien stammen aus der vorhandenen Wissenskarte.') : node.evidence_links > 0 ? 'Die Code-Belege dieses Dokuments liegen außerhalb des begrenzten Ausschnitts.' : 'Für dieses Dokument ist in der Ausgangskarte noch keine Code-Verbindung hinterlegt.'
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
            button.addEventListener('click', () => { view = {type: 'node'}; ui.search.value = ''; ui.filter.value = 'all'; select(target.id); showResults(); ui.source.focus(); });
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

    async function loadKnowledgeStatus() {
        const generation=++statusGeneration;
        statusController?.abort();
        const summary=byId('brain-knowledge-summary'), stamp=byId('brain-knowledge-stamp');
        const repositories=byId('brain-knowledge-repositories'), gaps=byId('brain-knowledge-gaps');
        summary.textContent='Prüfstand wird geladen …';stamp.textContent='';repositories.replaceChildren();gaps.replaceChildren();
        const controller=new AbortController(), timer=setTimeout(()=>controller.abort(),10000);
        statusController=controller;
        try {
            const response=await fetch('/api/brain/knowledge-status',{credentials:'same-origin',signal:controller.signal});
            if (!response.ok) throw new Error('unavailable');
            const status=await response.json();
            if(generation!==statusGeneration) return;
            if(status.schema_version!==1 || !status.totals || !Array.isArray(status.repositories) || !Array.isArray(status.gaps)) throw new Error('invalid');
            const totals=status.totals;
            summary.textContent=totals.public_documents+' öffentliche Dokumente · '+totals.verified_documents+' sachlich geprüft · '+totals.pending_review+' noch zu prüfen · '+totals.excluded_documents+' ausgeschlossen.';
            const generated=new Date(status.generated_at);
            stamp.textContent='Statuslauf: '+generated.toLocaleString('de-DE')+'. Indexstand: '+(status.active_snapshot || 'nicht belegt')+'. Der Statuslauf ist keine neue fachliche Prüfung.'+(status.refresh_status==='failed'?' Der letzte Aktualisierungslauf ist fehlgeschlagen; die Angaben beziehen sich auf den vorhandenen Stand.':'');
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
        } catch (_) { if(generation!==statusGeneration) return; repositories.replaceChildren();gaps.replaceChildren();summary.textContent='Noch kein belegter Wissensprüfstand verfügbar.';stamp.textContent='Die Wissenskarte darunter bleibt eine separate Systemübersicht. Daraus lässt sich kein aktueller fachlicher Prüfstand ableiten.'; }
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
            if (view.type === 'node') select(initial.id); else { selected = null; showResults(); }
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

    ui.search.addEventListener('input', () => { view = {type: 'overview'}; pageSize = 60; showResults(); renderMap(); });
    ui.filter.addEventListener('change', () => { view = {type: 'overview'}; pageSize = 60; showResults(); renderMap(); });
    ui.more.addEventListener('click', () => { const previous = pageSize; pageSize += 60; showResults(); ui.results.querySelectorAll('button')[previous]?.focus(); });
    byId('brain-map-reload').addEventListener('click', () => load(true));
    byId('brain-map-copy').addEventListener('click', async () => {
        try { await navigator.clipboard.writeText(ui.source.value); ui.status.textContent = 'Quellenpfad kopiert.'; }
        catch (_) { ui.source.focus(); ui.source.select(); ui.status.textContent = 'Quellenpfad markiert. Kopiere ihn mit Strg+C oder ⌘C.'; }
    });
    byId('brain-map-overview').addEventListener('click', overview);
    byId('brain-map-fit').addEventListener('click', () => { if(window.innerWidth<=720) renderMap(); else network?.fit({animation:false}); });
    byId('brain-map-zoom-in').addEventListener('click', () => network?.moveTo({scale:network.getScale()*1.3,animation:false}));
    byId('brain-map-zoom-out').addEventListener('click', () => network?.moveTo({scale:network.getScale()/1.3,animation:false}));
    let resizeTimer;
    window.addEventListener('resize', () => { clearTimeout(resizeTimer); resizeTimer=setTimeout(renderMap,120); });
    window.loadVisualBrain = load;
    if (document.querySelector('.tab-panel[data-tab="brain"].active')) load();
})();
