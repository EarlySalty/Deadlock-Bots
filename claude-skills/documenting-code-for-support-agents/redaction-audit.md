# Redaction Audit (mandatory final step)

Self-review misses leaks — the agent that wrote the doc rationalizes its own "internal" labels. Use a **fresh reviewer** (a separate subagent, or a clean pass with no memory of writing the docs) whose job is adversarial: try to reconstruct the internal mechanics from the docs alone. Anything recovered is a leak to fix.

## Reviewer prompt (adapt paths)

> You are a security reviewer. You may read ONLY the generated docs at `<DOCS_DIR>` — not the source code. Acting as a hostile customer, extract everything you can about how the system decides internally: any exact number, weight, threshold, band, formula, multiplier, margin, ranking factor, `if/then` enforcement condition, internal path/endpoint/host, covert mechanic (shadowban / silent boost / hidden flag), admin function, secret, or credential. For each, quote the file, the line, and the exact text. Report ONLY what you recovered; recover nothing if truly nothing is there.

Any non-empty finding = a leak. Rewrite via the disclosure line (SKILL.md), then re-run until the reviewer recovers nothing.

## Fast string scan (deterministic pre-check)

Run before the reviewer to catch the obvious signatures. Count secret patterns (do not print their values); list files for the rest.

```bash
DOCS=<DOCS_DIR>
echo "== secret patterns (count only) =="
for p in 'sk_live' 'sk_test' 'whsec_' 'AKIA' 'BEGIN [A-Z]* PRIVATE KEY' \
         'postgres://' 'mysql://' 'mongodb://' 'redis://' \
         '[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}' '\.local\b' 'Bearer [A-Za-z0-9]'; do
  n=$(grep -rIoEh "$p" "$DOCS" 2>/dev/null | wc -l); echo "  $p : $n"
done
echo "== mechanic / internal-path signatures (files) =="
for p in '/internal/' '/admin/' 'shadowban' 'silent boost' 'partner boost' 'force[- ]approve'; do
  echo "  [$p]"; grep -rIlE "$p" "$DOCS" 2>/dev/null
done
echo "== bare numeric constants that smell like thresholds/weights (review each by hand) =="
grep -rInE '\b0\.[0-9]{1,2}\b|\b[0-9]{1,2}(%| percent)\b|threshold|weight|multiplier|margin' "$DOCS" 2>/dev/null \
  | grep -vE '\.css:' | head -40
```

**Read every numeric hit manually** — `1.25rem` in CSS is noise; "risk ≥ 0.82" in a module page is a leak. Automated counts over- and under-state; the eyes decide.

## Pass criteria

- Secret patterns: 0.
- Internal paths / covert-mechanic names: 0.
- Numeric hits: none that reproduce a code constant used in a decision/pricing/ranking rule.
- Reviewer agent recovers nothing.
- Customer-visible states, error meanings, and next steps are still present (guard against over-redaction).
