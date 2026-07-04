---
name: documenting-code-for-support-agents
description: Use when generating documentation, a knowledge base, an FAQ, or a chatbot corpus from source code that AI support agents, chatbots, or customer-facing staff will read — especially when the code contains scoring, pricing, ranking, fraud/risk, matching, moderation, enforcement, or admin logic, secrets, or internal endpoints. Also use whenever asked to turn source code into customer- or support-facing documentation.
---

# Documenting Code for Support Agents

## Overview

**Core principle: document the observable behavior and the customer-relevant inputs — never the internal mechanism that produces them.**

The output is a knowledge base an AI support agent (or chatbot, or human support) reads to answer customers. Anything in the artifact can reach a customer via retrieval or prompt injection. So the redaction line is NOT "hide the secret parts from the customer-facing pages." It is: **sensitive quantities and mechanics never enter the artifact at all.**

**The one failure this skill exists to stop** (observed in testing): a capable agent redacts literal API keys perfectly, then faithfully transcribes every exact threshold, weight, margin, admin path, and covert enforcement mechanic into the docs — *including the support guide* — as long as it labels them "internal / never disclose."

> **A label is not redaction. "Internal-only" is not a container you may put secrets in.**
> If a fact cannot be safely stated to a customer, it does not enter the knowledge base — only that it *exists* and its *effect* may.

## When to Use

- Turning source code into a KB / FAQ / docs / chatbot corpus for AI support agents, chatbots, or customer-facing staff.
- The code contains any of: scoring, ranking, pricing/margin, fraud/abuse/risk, matching, moderation, enforcement, admin/operator tools, secrets, credentials, internal endpoints/URLs, non-public data sources.

**When NOT to use:** pure internal engineering docs for the dev team, behind auth, where the mechanism *is* the deliverable. If you are unsure who reads it, treat it as customer-reachable and use this skill.

## The Disclosure Line (the recipe)

For every sensitive element, state these three things and **stop**:

1. **That it exists** — "the system assigns an internal risk assessment to account actions."
2. **Its observable effect** — "high assessed risk can lead to review, limitation, or blocking."
3. **Customer-relevant inputs & next steps** — "verifying your account and keeping chargebacks low reduces risk; a limited account can appeal."

It NEVER contains: the number, weight, threshold, band, formula, multiplier, the internal ordering of conditions, the internal path/endpoint, or the name of a covert mechanic.

### Transform table (leaky → safe)

| In the code | ❌ Never write | ✅ Write instead |
|---|---|---|
| `AUTO_BAN_THRESHOLD = 0.82`, `REPORT_STRIKE_LIMIT = 3` | "risk ≥ 0.82 or 3 reports/24h ⇒ auto-block" | "Sufficiently high assessed risk, or repeated upheld reports, can lead to automatic limiting." |
| `FRAUD_WEIGHTS = {chargeback:0.35, …}` | "risk = 0.35·chargebacks + 0.25·age + 0.40·device" | "Risk is assessed from several internal signals, including account and payment history." |
| `BASE_MARGIN = 0.18`, `FLOOR_MARGIN = 0.07` | "18% base margin, 7% floor" | "Prices follow internal pricing rules and can vary with demand." |
| `BOOST_PARTNER = 1.25` in ranking | "partner sellers get a 1.25× boost" | Omit. At most "ranking uses several internal factors." **Do not reveal a non-public boost exists.** |
| `/internal/admin/force-approve`, `shadowban` | the path, or the covert-UI trick | Only the customer-visible outcome: "support can restore access after review." Never name the covert mechanic or its bypass. |
| `STRIPE_SECRET_KEY = …`, DSN, internal IPs | the value, even partially masked | Omit entirely. "Credentials are configured internally" if it must be mentioned at all. |

**Rule of thumb:** if a customer could use the fact to game, reverse-engineer, or manipulate the system, it stays out — *even labeled "internal."*

## Two Ways to Fail (both are failures)

- **Under-redaction** (the common one): transcribing exact values/mechanics and tagging them "internal." Fix: apply the disclosure line — abstract the value out of existence.
- **Over-redaction**: stripping so hard that support can't help. Fix: you MUST keep customer-observable states, error meanings, and concrete next steps. "Account limited" still needs "what limited means and how to appeal" — just not "you crossed 0.82."

## Rationalization Table — STOP if you think these

| Excuse | Reality |
|---|---|
| "I marked it internal / never-disclose, so including it is fine" | The KB feeds an AI agent. In-artifact = one injection or retrieval from the customer. Labels don't contain secrets; exclusion does. |
| "Developers need the exact number" | Devs read the source, not this KB. This artifact is for support/AI. Point devs to the code. |
| "It's already in the public repo, so it's not secret" | A plain-language, aggregated explanation of thresholds is a far bigger gift to an abuser than raw code. Abstract anyway. |
| "The agent needs the threshold to answer accurately" | The agent needs the *effect* and the *next step*, never the threshold. "High risk → limited → appeal" answers the customer; "≥ 0.82" only helps them game it. |
| "It's just a margin/weight, not a credential" | Pricing, margin, ranking, scoring internals ARE the listed sensitive categories. Same rule. |
| "The support guide is internal, so I can stash values there" | Testing showed exactly this leak. The support guide is customer-reachable too. Same disclosure line applies there. |

## Red Flags — you are about to leak

- You wrote a number, %, ratio, weight, or threshold copied from the code.
- You wrote an `if X and Y then block/ban/hide/price` condition in plain language.
- You named an internal path, endpoint, host, or a covert mechanic (shadowban, silent boost).
- You created an "internal only" section that contains exact values.
- Your `redaction-notes` / "what we omitted" list names a *specific* non-public mechanic (e.g. "excluded: the partner ranking boost"). **Naming what you excluded confirms it exists** — list generic categories only ("ranking factors were abstracted").

**All of these mean: apply the disclosure line and rewrite. Delete the value; keep the effect.**

## Workflow

1. Read the code. List modules, workflows, and customer-visible states/errors.
2. Flag every sensitive element by category (scoring, pricing, ranking, fraud/abuse/risk, matching, enforcement, admin, secrets, internal URLs).
3. For each, apply the disclosure line (exists + effect + next step; value excluded).
4. Emit the HTML. Structure, semantic-HTML rules, and the required agent-guide sections are in **html-scaffold.md** (same folder).
5. **Redaction audit (mandatory):** run the adversarial check in **redaction-audit.md** — a fresh reviewer tries to reconstruct internals from the docs alone; anything recovered is a leak to fix. Mark anything unclear from the code as "Unclear from the provided code."
6. Write a `redaction-notes` page listing the *categories* abstracted — never the secrets themselves.

## Real-World Impact

Baseline (no skill), 3 runs on a fixture with planted internals: literal secrets were redacted 3/3, but exact fraud thresholds, weights, margins, the `/internal/admin/force-approve` path, and the shadowban mechanic leaked in 3/3 — several straight into the support guide. This skill exists to close that gap.
