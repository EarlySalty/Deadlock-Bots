#!/usr/bin/env node
// Usage: node scripts/test_dashboard_ui.cjs /absolute/path/to/jsdom [chart-test-node_modules]
// Install the test-only DOM dependency outside the repo: npm install --prefix /tmp/ddc-ui-test jsdom
// Real Canvas QA: npm install --prefix /tmp/ddc-chart-test chart.js@4.4.6 canvas@3.2.0
// Pass /tmp/ddc-chart-test/node_modules as the optional second path; PNGs go beside that directory.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const {JSDOM, VirtualConsole} = require(process.argv[2] || 'jsdom');
const chartTestModules=process.argv[3];
const html = fs.readFileSync(path.join(__dirname, '../service/static/dashboard.html'), 'utf8');
const ID_A = '1411350229747241010';
const ID_B = '1118439769626648601';
const GUILD_A = '1289721245281292288';
const GUILD_B = '1289721245281292299';
const calls = [], errors = [], chartCalls = [];
const backend = fs.readFileSync(path.join(__dirname,'../rust/crates/dl-dashboard/src/web.rs'),'utf8');
const routes = [...backend.matchAll(/\.route\(\s*"([^"]+)"/g)].map(match=>new RegExp('^'+match[1].replace(/\{[^}]+\}/g,'[^/]+')+'$'));
let voiceDelay = false;
let searchFails = false;
let insightsFail = false;
let insightsDelay = false;
let authStatus = 200;
let mutationDenied = false;
let authResolvedAt = null;
let rateFixture = 'mixed';
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
function assertReadableChart({id,config},checkGeneratedLabels=true) {
    const options=config.options || {};
    const plugins=options.plugins || {};
    assert.equal(options.locale,'de-DE',id+': numbers use German formatting without relying on global defaults');
    assert.equal(plugins.legend?.labels?.color,'#e8e5dd',id+': legend text has an explicit light color');
    assert.ok(plugins.legend.labels.font?.size>=13,id+': legend font is at least 13px');
    for(const [axis,scale] of Object.entries(options.scales || {})) {
        assert.equal(scale.ticks?.color,'#c8c5bd',id+'/'+axis+': axis ticks remain readable');
        assert.ok(scale.ticks.font?.size>=13,id+'/'+axis+': ticks are at least 13px');
        assert.equal(scale.grid?.color,'rgba(255,255,255,0.14)',id+'/'+axis+': grid has visible contrast');
        if(scale.title?.display) {
            assert.equal(scale.title.color,'#e8e5dd',id+'/'+axis+': axis title remains readable');
            assert.ok(scale.title.font?.size>=14,id+'/'+axis+': axis title is at least 14px');
        }
    }
    assert.equal(plugins.tooltip?.backgroundColor,'#202126',id+': tooltip has an opaque dark surface');
    for(const part of ['title','body','footer']) {
        assert.equal(plugins.tooltip[part+'Color'],'#f5f2eb',id+': '+part+' tooltip text stays light');
        assert.ok(plugins.tooltip[part+'Font']?.size >= (part==='title'?14:13),id+': '+part+' tooltip font stays readable');
    }
    const generateLabels=plugins.legend.labels.generateLabels;
    if(generateLabels && checkGeneratedLabels) {
        const items=generateLabels({data:config.data,options,getDataVisibility:()=>true});
        assert.equal(items.length,config.data.labels.length,id+': custom legend covers every segment');
        for(const item of items) assert.equal(item.fontColor,'#e8e5dd',id+': generated legend items need their own Canvas fontColor');
    }
}
function copyChartConfig(value) {
    if(Array.isArray(value)) return value.map(copyChartConfig);
    if(value && typeof value==='object') return Object.fromEntries(Object.entries(value).map(([key,item])=>[key,copyChartConfig(item)]));
    return value;
}
function assertResolvedCharts(capturedCharts,poisonDefaults=false) {
    if(!chartTestModules) return;
    const {Chart,BasicPlatform,registerables}=require(path.join(chartTestModules,'chart.js'));
    const {createCanvas}=require(path.join(chartTestModules,'canvas'));
    const pinnedVersion=html.match(/chart\.js@([^/]+)/)?.[1];
    assert.equal(Chart.version,pinnedVersion,'Canvas QA uses the same pinned Chart.js version as the dashboard');
    Chart.register(...registerables);
    if(poisonDefaults) {
        Chart.defaults.color='#080808';Chart.defaults.font.size=5;
        Chart.defaults.plugins.legend.labels.color='#080808';
        Chart.defaults.plugins.legend.labels.font={size:5};
        Chart.defaults.plugins.tooltip.bodyColor='#080808';
        Chart.defaults.plugins.tooltip.bodyFont={size:5};
        Chart.defaults.scale.ticks.color='#080808';
        Chart.defaults.scale.ticks.font={size:5};
    }
    const outputDir=path.resolve(chartTestModules,'../chart-qa');
    fs.mkdirSync(outputDir,{recursive:true});
    const latestById=new Map(capturedCharts.map(chart=>[chart.id,chart]));
    for(const {id,config:captured} of latestById.values()) {
        const config=copyChartConfig(captured);
        config.platform=BasicPlatform;
        config.options={...config.options,responsive:false,animation:false,devicePixelRatio:1};
        config.plugins=[...(config.plugins||[]),{id:'testSurface',beforeDraw(chart){
            chart.ctx.save();chart.ctx.globalCompositeOperation='destination-over';
            chart.ctx.fillStyle='#15161b';chart.ctx.fillRect(0,0,chart.width,chart.height);chart.ctx.restore();
        }}];
        const canvas=createCanvas(config.type==='doughnut'?440:640,config.type==='doughnut'?285:240);
        const chart=new Chart(canvas,config);
        try {
            for(const item of chart.legend?.legendItems || []) {
                assert.equal(item.fontColor,'#e8e5dd',id+': actual Chart.js legend uses light Canvas text');
            }
            assertReadableChart({id,config:chart},false);
            if(config.type==='doughnut') assert.deepEqual(Object.keys(chart.scales),[],id+': applying a theme does not add Cartesian axes to donuts');
            if(id==='voice-hourly-chart') assert.equal(Object.keys(chart.scales).filter(axis=>axis.startsWith('y')).length,2,'Voice retains exactly its hours and peak axes');
            fs.writeFileSync(path.join(outputDir,id+'.png'),canvas.toBuffer('image/png'));
            if(config.type==='doughnut') {
                chart.resize(280,285);
                assert.ok(chart.legend.left>=0 && chart.legend.right<=280,id+': narrow-screen legend stays inside the canvas');
                assert.ok(chart.legend.bottom<=285,id+': narrow-screen legend stays vertically inside the canvas');
                assert.ok(chart.chartArea.height>=120,id+': narrow-screen legend leaves visible space for the donut');
                fs.writeFileSync(path.join(outputDir,id+'-narrow.png'),canvas.toBuffer('image/png'));
            }
        } finally {chart.destroy();}
    }
    console.log('PASS: real Chart.js '+Chart.version+' resolves '+latestById.size+' dashboard charts'+(poisonDefaults?' after foreign default mutations':'')+'; PNGs: '+outputDir);
}
function payload(url) {
    const uri = new URL(url, 'https://admin.example.test');
    const membershipBase = uri.searchParams.get('guild_id') === GUILD_B ? 900 : uri.searchParams.get('from') === '2026-09-01' ? 200 : 100;
    if (uri.pathname === '/api/auth/me') return {authenticated:true,csrf_token:'fixture-csrf-test-only',user:{display_name:'Testadmin'}};
    if (uri.pathname === '/api/user-retention') return {summary:{},candidates:[{user_id:ID_A,display_name:'Philipp <img src=x onerror=alert(1)>',days_inactive:14,last_active_at:'2026-09-01T12:00:00Z',membership_status:'left',last_message_status:'failed',last_message_at:'2026-09-02T12:00:00Z'}]};
    if (uri.pathname === '/api/voice-history') {
        if (uri.searchParams.has('search')) return {users:uri.searchParams.get('search') === 'nobody' ? [] : [{user_id:ID_A,display_name:'Alex <img src=x>'},{user_id:ID_B,display_name:'Alex <img src=x>'}]};
        const id=uri.searchParams.get('user_id');
        return {range_days:14,mode:uri.searchParams.get('mode')||'hour',buckets:[{label:'0',total_seconds:0,avg_peak:0},{label:'19',total_seconds:5400,avg_peak:2.5}],user:id?{user_id:id,display_name:id===ID_A?'Philipp':'Alex'}:null,user_summary:id?{user_id:id,display_name:id===ID_A?'Philipp':'Alex',range_seconds:3600,range_sessions:2}:null,recent_sessions:[]};
    }
    if (uri.pathname === '/api/server-stats') return {member_sources:{tracked_joins:3,known_joins:3,bucket_counts:{website:1,twitch:2}}};
    if (uri.pathname === '/api/co-player-network') return {nodes:[{id:ID_A,name:'Philipp <img src=x>'},{id:ID_B,name:'Alex'}],links:[{source:ID_A,target:ID_B,minutes:60,shared_seconds:3600,last_played:'2026-09-07T18:00:00Z'}],meta:{range_days:30,generated_at:'2026-09-08T10:00:00Z'}};
    if (uri.pathname === '/api/scrim-runtime-control') return {mode:'legacy',operational_writer:'dl-bots',epoch:1,updated_at:1788850000};
    if (uri.pathname === '/api/scrims') return {teams:[{id:1,name:'Erstes Team',members:[]},{id:2,name:'Zweites Team',members:[]}],matches:[{id:12,team_a_id:1,team_b_id:2,status:'planned',lobby_state:'planned',scheduled_at:1788850000}],participants:[],match_request_summaries:[],lagebilder:[]};
    if (uri.pathname === '/api/scrims/match-requests/defaults') return {templates:[],presets:[]};
    if (uri.pathname === '/api/reaction-roles') return [];
    if (uri.pathname === '/api/deadlock/heroes') return {heroes:[]};
    if (uri.pathname === '/api/audit-log') return {entries:[],total:0,noise_hidden:0};
    if (uri.pathname === '/api/repo-activity') return {available:true,generated_at:new Date().toISOString(),repos:[{name:'Diagramme – Prüfung',days:[{d:new Date().toISOString().slice(0,10),c:3,a:20,r:2,f:1}],total_commits:3}]};
    if (uri.pathname === '/api/brain/overview') return {report:{period_start:'2026-07-27T00:00:00Z',period_end:'2026-08-02T00:00:00Z',report_text:'Alter Bericht'},report_stale:true,effectiveness_week:{decisions:5028,shadow_decisions:5024,snapshots_created:4,measured_outcomes:16,successful_outcomes:0},plan_usage:{total:19,open:19,decided:0,commented:0},ledger_week:[],top_reasons:[],feeder_runs:[]};
    if (uri.pathname === '/api/brain/plan') return {run:null};
    if (uri.pathname === '/api/brain/wiki') return {pages:['wissen/Team.md'],index:'Siehe [[Team|Team-Wissen]].',log:'Letzter Lauf'};
    if (uri.pathname === '/api/brain/wiki/page') return {path:uri.searchParams.get('path'),content:'Gespeichertes Team-Wissen.'};
    if (uri.pathname.startsWith('/api/insights/')) {
        if (uri.pathname.endsWith('/import')) return {files:1,rows:1,results:[]};
        if (uri.pathname.endsWith('/overview')) return {live:{cards:[]},imported:[{import_kind:'membership',period_start:'2026-09-04',dimension:'total_membership',value:membershipBase},{import_kind:'membership',period_start:'2026-09-05',dimension:'total_membership',value:membershipBase+3},{import_kind:'retention',period_start:'2026-08-30',dimension:'pct_retained',value:12}],import_status:{rows:3,last_import_at:'2026-09-07T05:16:00Z',latest_period:'2026-09-05',kinds:['membership','retention']}};
        if(uri.pathname.endsWith('/growth')) return {live:{periods:[{period_start:'2026-09-05',joins:{website:3},leaves:{lt_1_month:1},member_total:103}],member_total_basis:'directory'}};
        if(uri.pathname.endsWith('/activation')) {
            const missing={period_start:'2026-02-01',data_available:false,same_day_interaction_rate:null,new_members:8,same_day_interaction_count:null};
            const zero={period_start:'2026-09-06',data_available:true,same_day_interaction_rate:null,new_members:0,same_day_interaction_count:0};
            return {live:{periods:rateFixture==='missing'?[missing]:rateFixture==='zero'?[zero]:[missing,{period_start:'2026-09-05',data_available:true,same_day_interaction_rate:0.5,new_members:4,same_day_interaction_count:2},zero]}};
        }
        if(uri.pathname.endsWith('/retention')) {
            const missing={cohort_week:'2026-02-02',data_available:false,retention_rate:null,joined:8,retained:null};
            const zero={cohort_week:'2026-08-24',data_available:true,retention_rate:null,joined:0,retained:0};
            return {live:{cohorts:rateFixture==='missing'?[missing]:rateFixture==='zero'?[zero]:[missing,{cohort_week:'2026-08-31',data_available:true,retention_rate:0.5,joined:4,retained:2}]}};
        }
        if(uri.pathname.endsWith('/engagement')) return {live:{periods:[{period_start:'2026-09-05',visitors:4,contributors:2,messages_total:13,avg_messages_per_contributor:6.5,voice_minutes_total:90}]}};
        if(uri.pathname.endsWith('/audience')) return {live:{member_tenure:{'<1_month':2,'1_6_months':1,'6_12_months':0,'1_year_plus':3},new_member_account_age_28d:{'<1_day':0,'<1_month':1,'1_month_plus':2}}};
        if(uri.pathname.endsWith('/top-invites')) return {live:{top_invites:[]}};
        return {live:{periods:[]}};
    }
    return {};
}
const virtualConsole = new VirtualConsole();
virtualConsole.on('jsdomError', error => errors.push(error.message));
const dom = new JSDOM(html.replace(/<script src=[^>]+><\/script>/g,''), {
    url:'https://admin.example.test/admin#insights',runScripts:'dangerously',pretendToBeVisual:true,virtualConsole,
    beforeParse(window) {
        window.setInterval = () => 1;
        window.matchMedia = () => ({matches:false,addListener(){},addEventListener(){}});
        window.HTMLElement.prototype.scrollIntoView = function(){};
        window.HTMLCanvasElement.prototype.getContext = function() { return {canvas:this}; };
        window.Chart = class { constructor(canvas,config) {assert.ok(canvas,'chart target exists');this.config=config;this.canvas=canvas;this.data=config.data;this.options=config.options;chartCalls.push({id:canvas.id||canvas.canvas?.id,config});} destroy(){} resize(){} update(){} };
        window.Chart.defaults={};
        window.fetch = async function(url,options={}) {
            calls.push({url:String(url),method:options.method||'GET',headers:options.headers,at:Date.now()});
            const uri=new URL(url,'https://admin.example.test');
            assert.ok(routes.some(route=>route.test(uri.pathname)), 'fixture request must exist in Rust router: '+uri.pathname);
            if (uri.pathname.startsWith('/api/insights/')) assert.ok([GUILD_A,GUILD_B].includes(uri.searchParams.get('guild_id')), 'every Insights request must carry an exact server ID');
            if (uri.pathname === '/api/auth/me') { await wait(180); authResolvedAt = Date.now(); }
            if (insightsDelay && uri.pathname.startsWith('/api/insights/') && uri.searchParams.get('from') === '2026-08-01') await wait(70);
            if (voiceDelay && uri.pathname==='/api/voice-history' && uri.searchParams.get('user_id')===ID_A) await wait(70);
            const fail=(uri.pathname === '/api/auth/me' && authStatus !== 200) || (mutationDenied && options.method === 'POST') || (searchFails && uri.searchParams.has('search')) || (insightsFail && uri.pathname.startsWith('/api/insights/'));
            const body=payload(url);
            return {ok:!fail,status:uri.pathname === '/api/auth/me' ? authStatus : (mutationDenied && options.method === 'POST') ? 403 : fail?503:200,statusText:fail?'Nicht verfügbar':'OK',headers:{get(){return null;}},async json(){return body;},async text(){return fail?'Vorübergehend nicht verfügbar':JSON.stringify(body);}};
        };
    },
});
const {window} = dom, {document} = window;
const $ = id => document.getElementById(id);
async function tab(name) {document.querySelector('.tab-btn[data-tab="'+name+'"]').click();await wait(20);}
(async()=>{
    await wait(60);
    assert.equal(document.querySelector('.tab-panel.active').dataset.tab,'insights','direct Insights link opens correct tab');
    assert.equal(document.querySelectorAll('.tab-panel').length,8);
    assert.equal(new Set([...document.querySelectorAll('[id]')].map(x=>x.id)).size,document.querySelectorAll('[id]').length,'all IDs unique');
    assert.equal(document.querySelectorAll('.tab-panel').length,document.querySelectorAll('#dashboard-main > .tab-panel').length,'panels share shell');
    const voiceChartBox=window.getComputedStyle($('voice-hourly-chart').parentElement);
    assert.equal(parseFloat(voiceChartBox.minHeight),0,'a legacy minimum height cannot override the compact Voice chart');
    assert.ok(parseFloat(voiceChartBox.height)<=320,'Voice chart has a bounded height');
    assert.equal($('insights-activation').parentElement,$('insights-retention').parentElement,'related activation and retention panels share one responsive row');
    assert.ok(parseFloat(window.getComputedStyle($('insights-activationChart').parentElement).height)<=240,'single-rate chart stays compact');
    const engagementGrid=$('insights-engagement').querySelector('.grid.three');
    assert.equal(window.getComputedStyle(engagementGrid).flexWrap,'wrap');
    assert.ok([...engagementGrid.children].every(card=>parseFloat(window.getComputedStyle(card).flexGrow)>0),'engagement cards fill the available row instead of leaving a half-empty last row');
    assert.ok(!html.includes('member-source-twitch-links'),'unused Twitch links removed');
    assert.ok(!html.includes('d3@'),'removed graph dependency');
    await tab('scrims');
    $('scrim-team-a').value='1';$('scrim-team-b').value='2';
    $('scrim-create-form').dispatchEvent(new window.Event('submit',{bubbles:true,cancelable:true}));
    assert.ok(!calls.some(call=>call.method==='POST'),'mutation waits while initial authentication is loading');
    await wait(180);
    const mutation=calls.find(call=>call.url==='/api/scrims/matches' && call.method==='POST');
    assert.ok(mutation,'real Scrim form submits');assert.equal(mutation.headers['X-CSRF-Token'],'fixture-csrf-test-only','CSRF comes from initial auth/me');assert.ok(mutation.at>=authResolvedAt);
    assert.equal(calls.filter(call=>call.url==='/api/auth/me').length,1,'all views share a single auth request');
    mutationDenied=true;$('scrim-create-form').dispatchEvent(new window.Event('submit',{bubbles:true,cancelable:true}));await wait(30);assert.match($('scrim-message').textContent,/Berechtigung|Sitzung/);mutationDenied=false;
    await tab('insights');
    const imported=chartCalls.find(x=>x.id==='insights-importedChart');
    assert.match($('insights-activation-counts').textContent,/2 von 4/,'activation numerator and denominator are visible without hovering');
    assert.match($('insights-retention-counts').textContent,/2 von 4/,'retention numerator and denominator are visible without hovering');
    assert.equal($('insights-activation-values').querySelectorAll('tbody tr').length,3,'all periods retain their counts, including zero-member periods');
    for(const target of ['activation','retention']) {
        const missingRow=$('insights-'+target+'-values').querySelector('tbody tr:last-child');
        assert.equal(missingRow.children[1].textContent,'8','known join denominator survives missing activity data');
        assert.equal(missingRow.children[2].textContent,'Nicht verfügbar','null activity counts must not become zero');
        assert.match(missingRow.children[3].textContent,/Keine vollständigen Rohdaten/);
        assert.match($('insights-'+target+'-counts').textContent,/Rohdaten/,'missing periods are visible outside the collapsed detail table');
        const config=chartCalls.filter(chart=>chart.id==='insights-'+target+'Chart').at(-1).config;
        assert.equal(config.data.datasets[0].data[0],null,'unavailable rates stay chart gaps');
        assert.equal(config.data.datasets[0].spanGaps,false,'missing raw data is not interpolated across');
        const tooltip=config.options.plugins.tooltip.callbacks.label({dataIndex:0,parsed:{y:null}}).join(' ');
        assert.match(tooltip,/Keine vollständigen Rohdaten/);
        assert.doesNotMatch(tooltip,/0 %|aktiviert: 0|Zurückgekehrt: 0/);
    }
    rateFixture='missing';$('insights-refreshInsights').click();await wait(25);
    for(const target of ['activation','retention']) assert.match($('insights-'+target+'-counts').textContent,/Keine vollständigen Rohdaten/,'an entirely unavailable period explains why no rate is shown');
    rateFixture='zero';$('insights-refreshInsights').click();await wait(25);
    for(const target of ['activation','retention']) {
        assert.match($('insights-'+target+'-counts').textContent,/Keine Beitritte/,'real zero-member periods have their own empty state');
        const cells=$('insights-'+target+'-values').querySelector('tbody tr').children;
        assert.equal(cells[1].textContent,'0');assert.equal(cells[2].textContent,'0');
        assert.match(cells[3].textContent,/Keine Beitritte/);
        assert.doesNotMatch($('insights-'+target+'-values').textContent,/Rohdaten|Nicht verfügbar/);
    }
    rateFixture='mixed';$('insights-refreshInsights').click();await wait(25);
    assertResolvedCharts(chartCalls);
    for(const chart of chartCalls) assertReadableChart(chart);
    assert.deepEqual(Array.from(imported.config.data.datasets[0].data),[100,103],'member totals are charted as snapshots, not summed');
    assert.ok($('insights-importStatus').textContent.includes('5.9.2026'));
    window.Chart.defaults={color:'#080808',font:{size:5},plugins:{legend:{labels:{color:'#080808',font:{size:5}}},tooltip:{bodyColor:'#080808',bodyFont:{size:5}}},scales:{linear:{ticks:{color:'#080808',font:{size:5}}}}};
    const beforeDefaultMutation=chartCalls.length;
    $('insights-from').value='2026-08-01';$('insights-from').dispatchEvent(new window.Event('change'));await wait(20);
    assert.ok(calls.some(x=>x.url.includes('/api/insights/overview?')&&x.url.includes('from=2026-08-01')&&!x.url.includes('insights-from')),'date filter reaches backend');
    insightsDelay=true;
    $('insights-from').value='2026-08-01';$('insights-from').dispatchEvent(new window.Event('change'));
    assert.equal(window.getComputedStyle(document.querySelector('.insights-panel')).opacity,'0.55','the panel visibly dims during a pending Insights request');
    $('insights-from').value='2026-09-01';$('insights-from').dispatchEvent(new window.Event('change'));await wait(100);
    assert.equal(document.querySelector('.insights-panel').classList.contains('loading'),false,'the loading state ends after the current request');
    const newestImport=chartCalls.filter(chart=>chart.id==='insights-importedChart').at(-1);
    assert.deepEqual(Array.from(newestImport.config.data.datasets[0].data),[200,203],'slower old Insights range cannot replace new range');
    insightsDelay=false;
    const insightsReads=()=>calls.filter(call=>call.method==='GET' && call.url.startsWith('/api/insights/'));
    $('insights-guildId').value=GUILD_B;$('insights-guildId').dispatchEvent(new window.Event('change'));await wait(20);
    assert.deepEqual(Array.from(chartCalls.filter(chart=>chart.id==='insights-importedChart').at(-1).config.data.datasets[0].data),[900,903],'second server shows its own snapshots');
    assert.equal(insightsReads().slice(-7).every(call=>new URL(call.url,'https://admin.example.test').searchParams.get('guild_id')===GUILD_B),true,'all seven read endpoints follow the selected server');
    const importInput=$('insights-fileInput');
    Object.defineProperty(importInput,'files',{value:[new window.File(['date,members\n2026-09-05,903'],'insights.csv',{type:'text/csv'})],configurable:true});
    importInput.dispatchEvent(new window.Event('change'));await wait(30);
    const upload=calls.find(call=>call.url.startsWith('/api/insights/import?') && call.method==='POST');
    assert.equal(new URL(upload.url,'https://admin.example.test').searchParams.get('guild_id'),GUILD_B,'upload uses the same exact server ID');
    assert.equal(insightsReads().slice(-7).every(call=>new URL(call.url,'https://admin.example.test').searchParams.get('guild_id')===GUILD_B),true,'refresh after upload stays within its server');
    for(const invalid of ['', 'not-an-id', '9223372036854775808']) {
        const before=insightsReads().length;
        $('insights-guildId').value=invalid;$('insights-guildId').dispatchEvent(new window.Event('change'));await wait(20);
        assert.equal(insightsReads().length,before,'invalid or overflowing server IDs never issue unfiltered requests');
        assert.match($('insights-importStatus').textContent,/gültige Server-ID/);assert.equal($('insights-importMetric').disabled,true);
    }
    $('insights-guildId').value=GUILD_A;$('insights-guildId').dispatchEvent(new window.Event('change'));await wait(20);
    assert.deepEqual(Array.from(chartCalls.filter(chart=>chart.id==='insights-importedChart').at(-1).config.data.datasets[0].data),[200,203],'switching back restores only the first server');
    await tab('activity');
    $('voice-history-user').value='Alex';$('voice-user-apply').click();await wait(20);
    let results=[...document.querySelectorAll('.search-result')];assert.equal(results.length,2,'duplicate names remain distinct results');
    assert.ok(results[0].textContent.includes(ID_A));assert.ok(results[1].textContent.includes(ID_B));assert.equal($('voice-search-results').querySelectorAll('img').length,0,'search names are text, not HTML');
    results[1].click();await wait(20);assert.ok(calls.some(x=>x.url.includes('user_id='+ID_B)),'snowflake transmitted as exact string');
    assert.ok($('voice-user-summary').textContent.includes(ID_B));
    assert.ok($('co-network-list').textContent.includes('Philipp <img src=x>'));assert.equal($('co-network-list').querySelectorAll('img').length,0);
    assert.ok(!$('co-network-list').textContent.includes('Sessions'),'poll counters no longer sold as sessions');
    voiceDelay=true;
    window.selectVoiceMember({user_id:ID_A,display_name:'Philipp'});window.selectVoiceMember({user_id:ID_B,display_name:'Alex'});await wait(100);
    assert.ok($('voice-user-summary').textContent.includes(ID_B),'slower previous member response cannot overwrite selection');
    $('voice-history-user').value='nobody';$('voice-user-apply').click();await wait(20);assert.match($('voice-search-results').textContent,/Kein Mitglied/);
    searchFails=true;$('voice-history-user').value='Alex';$('voice-user-apply').click();await wait(20);assert.match($('voice-search-results').textContent,/Suche fehlgeschlagen/);searchFails=false;
    $('voice-user-reset').click();await wait(20);assert.equal($('voice-history-user').value,'');
    await tab('overview');assert.ok($('retention-recent').textContent.includes('Ausgetreten'));assert.ok($('retention-recent').textContent.includes('Versand fehlgeschlagen'));assert.equal($('retention-recent').querySelectorAll('img').length,0);
    await tab('scrims');assert.ok($('scrim-match-rows').textContent.includes('Erstes Team vs Zweites Team'));assert.ok($('scrim-match-rows').textContent.includes('Geplant'));assert.equal($('scrim-runtime-control').tagName,'DETAILS');assert.equal($('scrim-runtime-control').open,false);assert.equal(document.querySelector('.scrim-block-card').open,false);
    await tab('brain');assert.equal($('brain-verdict').textContent,'Ein Nutzen ist bisher nicht belegt');assert.ok($('brain-report-period').textContent.includes('Veraltet'));assert.ok($('brain-plan-usage').textContent.includes('bewertet: 0'));
    $('brain-wiki-nav').closest('details').open=true;
    const wikiIndex=$('brain-wiki-nav').querySelector('[data-special="index"]');
    assert.equal(wikiIndex.tagName,'BUTTON');assert.equal(wikiIndex.type,'button');assert.equal(wikiIndex.tabIndex,0,'wiki navigation uses native keyboard controls');
    wikiIndex.focus();assert.equal(document.activeElement,wikiIndex);wikiIndex.click();
    const wikiLink=$('brain-wiki-content').querySelector('.wiki-link');
    assert.equal(wikiLink.tagName,'BUTTON');assert.equal(wikiLink.tabIndex,0,'inline wiki links are keyboard reachable');
    wikiLink.focus();assert.equal(document.activeElement,wikiLink);wikiLink.click();await wait(20);
    assert.ok($('brain-wiki-content').textContent.includes('Gespeichertes Team-Wissen.'));assert.equal(document.activeElement,$('brain-wiki-content').querySelector('h3'),'new wiki page receives focus');
    assert.equal($('brain-wiki-nav').querySelector('[data-page]').getAttribute('aria-current'),'page');
    for(const target of ['brain','insights']) {
        await tab(target);document.querySelector('.skip-link').click();await wait(10);
        assert.equal(document.querySelector('.tab-panel.active').dataset.tab,target,'skip link preserves current tab');assert.equal(window.location.hash,'#'+target);assert.equal(document.activeElement,$('dashboard-main'));
    }
    window.location.hash='#dashboard-main';await wait(20);assert.equal(document.querySelector('.tab-panel.active').dataset.tab,'insights','content anchors do not change tabs');
    for(const name of ['deadlock','audit','entwicklung','overview']){await tab(name);assert.equal(document.querySelector('.tab-panel.active').dataset.tab,name);assert.equal(window.location.hash,'#'+name);}
    $('server-stats-refresh').click();await wait(20);
    const refreshedCharts=chartCalls.slice(beforeDefaultMutation);
    for(const canvas of document.querySelectorAll('canvas')) {
        assert.ok(refreshedCharts.some(chart=>chart.id===canvas.id),'fixture exercises '+canvas.id+' after another tab changes Chart.defaults');
    }
    for(const chart of refreshedCharts) assertReadableChart(chart);
    assertResolvedCharts(refreshedCharts,true);
    const voiceChart=refreshedCharts.filter(chart=>chart.id==='voice-hourly-chart').at(-1);
    const voiceTooltip=voiceChart.config.options.plugins.tooltip.callbacks.label;
    assert.match(voiceTooltip({dataset:{label:'Sprachzeit',yAxisID:'yHours'},parsed:{y:1.25}}),/1,25/,'Voice tooltip formats decimal hours in German');
    assert.match(voiceTooltip({dataset:{label:'Spitze',yAxisID:'yPeak'},parsed:{y:2.5}}),/2,5/,'Voice tooltip formats decimal peaks in German');
    await tab('insights');insightsFail=true;$('insights-refreshInsights').click();await wait(30);assert.ok($('insights-importStatus').textContent.includes('Daten konnten nicht geladen werden'));assert.ok(!document.querySelector('.insights-panel').classList.contains('loading'));assert.equal($('insights-importMetric').disabled,true);assert.equal($('insights-importedValues').textContent,'','failed current range cannot reuse old values');
    insightsFail=false;await tab('scrims');
    authStatus=401;window.eval('authReady = null; csrfToken = "";');
    $('scrim-team-a').value='1';$('scrim-team-b').value='2';$('scrim-create-form').dispatchEvent(new window.Event('submit',{bubbles:true,cancelable:true}));await wait(210);
    assert.match($('scrim-message').textContent,/melde dich|Anmeldung/i,'expired session has useful feedback');
    assert.deepEqual(errors,[],'no unhandled DOM/script errors');
    const nonRead=calls.filter(call=>call.method!=='GET');assert.equal(nonRead.length,3,'only the two explicit Scrim submits and CSV fixture upload mutate');
    assert.ok(calls.every(call=>!/^\/api\/(status|cogs|logs|standalone|bot\/restart|dashboard\/restart)/.test(call.url)),'removed Legacy routes are never called');
    console.log('PASS: native Insights, date filters, snapshots, all 8 tabs, initial Auth→Scrim mutation/CSRF, 401/403 errors, Rust-route inventory, Insights race, exact IDs, name duplicates/XSS, stale request guard, search errors, AFK membership, Scrims, Brain and load failure.');
    window.close();
})().catch(error=>{console.error(error);console.error('DOM errors:',errors);window.close();process.exitCode=1;});
