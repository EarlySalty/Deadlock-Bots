// UI smoke test against synthetic Rust-generated contracts only. No live API.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
// Usage: node scripts/test_operating_config_ui.cjs <fixtures.json> <playwright-core-directory> <output-directory> <chromium-binary>
// fixtures.json: {"discord": <dl-core example output>, "steam": <steam-config example output>}.
if (process.argv.length !== 6) {
    throw new Error('Aufruf: node scripts/test_operating_config_ui.cjs fixtures.json /pfad/playwright-core /pfad/testausgabe /pfad/chromium');
}
const { chromium } = require(path.resolve(process.argv[3]));
const root = path.resolve(process.argv[4]);
fs.mkdirSync(root, { recursive: true });
const source = fs.readFileSync(path.join(__dirname,'../service/static/operating-config.js'),'utf8');
const dashboard = fs.readFileSync(path.join(__dirname,'../service/static/dashboard.html'),'utf8');
const bundle = { styles: [...dashboard.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(match=>match[1]).join('\n') };
const fixtures = JSON.parse(fs.readFileSync(path.resolve(process.argv[2]),'utf8'));
const requests = [];
const errors = [];
const results = [];
let conflict = false;
let malformedSnapshot = false;
const record = name => { results.push(name); fs.writeFileSync(path.join(root,'browser-qa.json'),JSON.stringify({state:'running',passed:results,errors},null,2)); };
const html = `<!doctype html><html lang="de"><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Admin – isolierte Testansicht</title><style>${bundle.styles}</style><style>body{margin:0;background:#0b0c0e;color:#eee;font-family:Arial,sans-serif}main{max-width:1320px;margin:0 auto;padding:24px}h1{font-size:28px}.qa-note{font-size:12px;color:#b9af97;margin:4px 0 20px}button{background:#302b20;border:1px solid #78623c;color:#eee;border-radius:5px;cursor:pointer}button:disabled{opacity:.4;cursor:default}</style><main><h1>Betriebseinstellungen</h1><p class="qa-note">ISOLIERTE TESTANSICHT · Synthetische Daten · Keine laufenden Bots verändert</p><div id="operating-config-cards"></div></main><script>async function fetchJSON(url, options={}){const response=await fetch(url,{...options,headers:{'Content-Type':'application/json'}});const value=await response.json();if(!response.ok)throw new Error(value.error||'Fehler '+response.status);return value;}</script><script src="/api/admin/betriebskonfiguration.js"></script></html>`;
(async () => {
    const browser = await chromium.launch({executablePath:path.resolve(process.argv[5]),headless:true,timeout:20000,args:['--no-sandbox','--disable-dev-shm-usage']});
    const page = await browser.newPage({viewport:{width:1440,height:1080},deviceScaleFactor:1});
    page.setDefaultTimeout(12000);
    page.on('pageerror', error => errors.push(error.message));
    page.on('dialog', dialog => dialog.accept());
    await page.route('**/*', async route => {
        const req=route.request();const url=new URL(req.url());
        if(url.pathname==='/')return route.fulfill({status:200,contentType:'text/html',body:html});
        if(url.pathname.endsWith('betriebskonfiguration.js'))return route.fulfill({status:200,contentType:'text/javascript',body:source});
        const key=url.pathname==='/api/admin/betriebskonfiguration'?'discord':url.pathname==='/api/admin/steam-betriebskonfiguration'?'steam':null;
        if(!key)return route.fulfill({status:404,body:''});
        if(req.method()==='PATCH'){
            const body=req.postDataJSON();requests.push({key,body});
            if(conflict)return route.fulfill({status:409,json:{error:'Die Datei wurde inzwischen geändert.'}});
            assert.equal(body.revision,fixtures[key].revision);
            for(const [name,value] of Object.entries(body.changes)) fixtures[key].catalog.values[name]=value;
            fixtures[key].revision=(key==='discord'?'e':'f').repeat(64);
            if(key==='discord') fixtures[key].services.forEach(service=>service.restart_required=true);
            else fixtures[key].active.forEach(service=>service.state='restart_required');
        }
        if (malformedSnapshot && key === 'discord' && req.method() === 'GET') {
            return route.fulfill({status:200,json:{...fixtures[key], services:null}});
        }
        return route.fulfill({status:200,json:fixtures[key]});
    });
    try {
        await page.goto('http://admin-settings.test/#betrieb',{waitUntil:'domcontentloaded',timeout:20000});
        const discord=page.locator('[data-service="discord"]');const steam=page.locator('[data-service="steam"]');
        await discord.getByText('167 änderbar · 53 geschützt',{exact:true}).waitFor();
        await steam.locator('.settings-count').first().filter({hasText:'änderbar'}).waitFor();
        assert.equal(await discord.locator('[type=submit]').isDisabled(),true);
        assert.equal(await steam.locator('[type=submit]').isDisabled(),true);
        assert.equal(await discord.locator('input[name],select[name],textarea[name]').count(),167);
        assert.equal(await steam.locator('input[name],select[name],textarea[name]').count(),fixtures.steam.catalog.fields.filter(field=>field.writable).length);
        assert.equal(errors.length,0);
        record('Alle 220 Discord-Felder und alle Steam-Felder ohne JavaScript-Fehler gerendert; kein falscher Entwurf nach dem Laden.');
        await page.screenshot({path:path.join(root,'admin-settings-desktop.png'),fullPage:false,timeout:30000});
        await discord.getByRole('button',{name:'DeepSeek V4.1 Flash für Fireworks wählen',exact:true}).click();
        await discord.locator('details.settings-diff').waitFor({state:'visible'});
        assert.equal(await page.locator('#setting-discord-llm-fireworks-model').inputValue(),'accounts/fireworks/models/deepseek-v4p1-flash');
        assert.equal(await discord.locator('[type=submit]').isEnabled(),true);
        await discord.locator('[type=submit]').click();
        await discord.getByText('Änderungen gespeichert. Kein Dienst wurde automatisch neu gestartet.',{exact:true}).waitFor();
        const preset=requests.at(-1).body.changes;
        assert.equal(preset['llm.use_cases.faq.model'],'accounts/fireworks/models/deepseek-v4p1-flash');
        assert.equal(Object.hasOwn(preset,'llm.use_cases.voice_hint.model'),false);
        assert.equal(Object.keys(preset).some(key=>key.endsWith('.provider')),false);
        assert.equal(Object.keys(preset).length,12);
        record('V4.1-Voreinstellung aktualisiert Fireworks und elf Fireworks-Anwendungsfälle, aber weder OpenAI-Zuordnungen noch andere Werte.');
        await discord.locator('.settings-search').fill('concierge_pate_channel_id');
        const role=page.locator('#setting-discord-runtime-community-concierge_pate_channel_id');
        await role.fill('1547199955133927465');
        await discord.locator('[type=submit]').click();
        await discord.getByText('Änderungen gespeichert. Kein Dienst wurde automatisch neu gestartet.',{exact:true}).waitFor();
        assert.deepEqual(requests.at(-1).body.changes,{'runtime.community.concierge_pate_channel_id':'1547199955133927465'});
        record('Einzelfeldänderung sendet nur den geänderten Pfad; 19-stellige ID bleibt bytegenau als Dezimalstring erhalten.');
        conflict=true;
        await role.fill('1547199955133927466');await discord.locator('[type=submit]').click();
        await discord.locator('.settings-error').filter({hasText:'Dein Entwurf bleibt erhalten'}).waitFor();
        assert.equal(await role.inputValue(),'1547199955133927466');
        assert.equal(await discord.locator('[type=submit]').isEnabled(),true);
        record('409-Konflikt zeigt Fehler und erhält den Entwurf einschließlich exakter ID.');
        conflict=false;
        malformedSnapshot=true;
        await discord.getByRole('button',{name:'Entwurf verwerfen und neu laden',exact:true}).click();
        await discord.locator('.settings-error').filter({hasText:'Dienstantwort ist unvollständig'}).waitFor();
        assert.equal(await role.inputValue(),'1547199955133927466');
        assert.equal(await discord.locator('[type=submit]').isEnabled(),true);
        record('Eine unvollständige HTTP-200-Antwort verwirft weder den Entwurf noch die geladenen Eingabefelder.');
        malformedSnapshot=false;
        await discord.getByRole('button',{name:'Entwurf verwerfen und neu laden',exact:true}).click();
        await page.waitForFunction(()=>document.querySelector('#setting-discord-runtime-community-concierge_pate_channel_id')?.value==='1547199955133927465');
        await discord.locator('.settings-search').fill('concierge_proactive');
        const proactive=page.locator('#setting-discord-runtime-community-concierge_proactive');
        assert.equal(await proactive.inputValue(),'false');
        await proactive.selectOption('');await discord.locator('[type=submit]').click();
        await page.waitForFunction(()=>document.querySelector('[data-service=discord] [type=submit]').disabled);
        assert.deepEqual(requests.at(-1).body.changes,{'runtime.community.concierge_proactive':null});
        record('Dienststandard entfernt optionalen booleschen Override; false wird nicht in null umgedeutet.');
        await discord.locator('.settings-search').fill('');
        await page.setViewportSize({width:390,height:844});
        await page.evaluate(()=>window.scrollTo(0,0));
        assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),true);
        await page.screenshot({path:path.join(root,'admin-settings-mobile.png'),fullPage:false,timeout:30000});
        record('Mobile Ansicht bei 390 px ohne horizontalen Überlauf.');
        assert.equal(errors.length,0);
        fs.writeFileSync(path.join(root,'browser-qa.json'),JSON.stringify({state:'passed',passed:results,errors,requests:requests.length,fields:{discord:fixtures.discord.catalog.fields.length,steam:fixtures.steam.catalog.fields.length}},null,2));
        console.log('Browser QA passed');
    } finally { await browser.close(); }
})().catch(error=>{fs.writeFileSync(path.join(root,'browser-qa.json'),JSON.stringify({state:'failed',passed:results,errors:[...errors,error.stack]},null,2));console.error(error.stack);process.exitCode=1;});
