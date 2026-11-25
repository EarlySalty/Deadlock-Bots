const fs = require("fs");
const crypto = require("crypto");
const dir = "/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-07-concierge-coaching-fallback/";
const raw = fs.readFileSync("/home/nathanael/Documents/.tasks/2026-10-02-berichte-einbau/docs-private/import-reviewed-v1/provenance/core-documents-public.json");
if (crypto.createHash("sha256").update(raw).digest("hex") !== "3c33bc8340674d0fa6fdeadfd0a565c352b6a5274774ad4c3f2970e81f7b5132") {
    throw Error("original bundle mismatch");
}
const old = JSON.parse(raw);
const config = JSON.parse(fs.readFileSync("/etc/deadlock-brain/maintenance.json", "utf8"));
const runtime = JSON.parse(fs.readFileSync("/etc/deadlock-brain/maintenance-runtime.json", "utf8"));
const targets = ["public/discord-server/dm-concierge.html", "public/discord-server/paten.html"];
const bots = config.repositories.find(x => x.id === "Deadlock-Bots");
const docs = config.repositories.find(x => x.id === "Deadlock-Docs");
for (const target of targets) {
    const policy = old.find(d => d.logical_id === target).origin.policy;
    if (policy.visibility !== "public" || !policy.publication_allowed || !policy.provider_egress_allowed || policy.raw_retention_allowed || JSON.stringify(policy.allowed_scopes) !== JSON.stringify(["bot.public"])) {
        throw Error("unexpected old policy");
    }
    docs.doc_targets = docs.doc_targets.filter(x => x !== target);
    delete docs.target_document_policies[target];
    bots.doc_targets = bots.doc_targets.concat([target]);
    bots.target_document_policies[target] = policy;
}
const path = dir + "maintenance-restore-legacy-20261008.json";
fs.writeFileSync(path, JSON.stringify(config, null, 2) + "\n", {flag: "wx", mode: 0o600});
runtime.maintenance_config = path;
fs.writeFileSync(dir + "maintenance-restore-legacy-20261008.runtime.json", JSON.stringify(runtime, null, 2) + "\n", {flag: "wx", mode: 0o600});
console.log("Prepared restoration of the exact two previous public policies; canonical files unchanged.");
