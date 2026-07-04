# HTML Scaffold & Output Contract

Reference for the mechanical structure. The judgment (what to redact) lives in SKILL.md — apply the disclosure line to every page below.

## Folder structure

```
/docs
  /index.html                     project overview, safe-abstraction architecture, nav
  /modules/<module>.html          one page per relevant module
  /workflows/<workflow>.html      one page per important flow
  /support/agent-guide.html       MOST IMPORTANT — see required sections below
  /support/troubleshooting.html   structured first-aid, not FAQ
  /reference/glossary.html        consistent term definitions
  /reference/states-and-errors.html  statuses, error codes, customer meaning
  /security/redaction-notes.html  GENERIC categories abstracted — never name a specific
                                  non-public mechanic; naming what you excluded confirms it exists
```

Emit the folder tree first, then the content of the key files. Minimal inline CSS only; **no** external scripts, fonts, CDNs, analytics, or JavaScript.

## Per-page rules

- One clear `<h1>`; a short summary `<p>` at the top of every page.
- Semantic HTML: `<main> <section> <article> <nav> <h1>-<h3> <p> <ul> <table>`.
- Speaking anchor IDs on important sections (`id="account-limited"`).
- Tables for module overviews, statuses, errors, and support do/don't.
- Mark anything you cannot determine from the code: **"Unclear from the provided code."**

## Minimal page skeleton

```html
<main>
  <h1>Account Statuses</h1>
  <p><strong>Summary.</strong> The states a customer or support agent may see, what
     each means, and the safe next step.</p>
  <section id="limited">
    <h2>Limited</h2>
    <table>
      <tr><th>What the customer sees</th><td>Some actions are temporarily restricted.</td></tr>
      <tr><th>What it means</th><td>The account is under a systemic restriction pending review.</td></tr>
      <tr><th>Safe next step</th><td>The customer can appeal; support can escalate for review.</td></tr>
      <tr><th>Agent must NOT say</th><td>Any internal trigger, score, threshold, or condition.</td></tr>
    </table>
  </section>
</main>
```

## Module page — required fields

Name · purpose · business responsibility · when it's used · conceptual inputs · conceptual outputs · relevant states · known errors/edge cases · dependencies · notes for AI/support · **"internal details deliberately abstracted"** (categories only, no values).

## Workflow page — required fields

Trigger · modules involved · business flow (steps) · user actions · system reactions · possible outcomes · typical errors · safe support notes · **what an agent MAY say** · **what an agent MUST NOT say**.

## agent-guide.html — required sections (this page is customer-reachable; redact it too)

1. **May say to customers** — effects, visible states, next steps, appeal paths.
2. **Internal-use only** — kept minimal; still no exact values or mechanics.
3. **Never say** — any internal value, formula, threshold, ranking/pricing logic, admin path, covert mechanic, secret, or infra detail.
4. **Uncertain cases** — abstract and, if not safely explainable, escalate to human support.
5. **Safe answer patterns**, e.g.:
   - "The system uses internal criteria for this decision. The exact logic isn't public, but I can explain which visible factors matter for you."
   - "I can't reveal the internal checks, but I can help you understand the next possible steps."
   - "This is a system-side assessment. If you think it's wrong, the case can be reviewed or escalated to support."
6. **Escalation triggers** — when the agent must hand off to a human.
7. **First-aid steps** the agent may safely recommend.
