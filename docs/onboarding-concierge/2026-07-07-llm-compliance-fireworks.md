# LLM-Compliance-Nachweis: Fireworks (DeepSeek) für die Concierge-DM-Persona

Anforderung aus Konzept §5.6: Prozessor mit DPA + No-Training-Zusage
(Discord-Policy-konform), dokumentiert. Stand 2026-07-07.

## Belege (Quelle: https://docs.fireworks.ai/guides/security_compliance/data_security)

- **Zero Data Retention:** „Fireworks does not log or store prompt or
  generation data for open models, without explicit user opt-in."
  Damit keine Speicherung und kein Training auf User-DM-Inhalten.
  Detail-Policy: https://docs.fireworks.ai/guides/security_compliance/data_handling
- **Verschlüsselung:** TLS 1.2+ in Transit, AES-256 at Rest.
- **Löschbarkeit:** Kundendaten aus aktiven Workflows permanent löschbar
  mit auditierbarer Bestätigung.
- **Access Logging / Workload Isolation:** dokumentiert auf derselben Seite.
- **Trust Center** (Audit-Reports, Vertragsdokumente):
  https://trust.fireworks.ai/

## Detail-Belege (ZDR-Policy, Stand 2026-07-07)

- ZDR ist Default: Prompt- und Antwortdaten existieren nur im flüchtigen
  Speicher für die Dauer des Requests (bei Prompt-Caching einige Minuten
  KV-Cache im RAM), keine persistente Speicherung, kein Logging ohne
  explizites Opt-in.
- **Ausnahme Response API:** Bei `store=True` (dort Default) werden
  Konversationen 30 Tage gespeichert. **Leitplanke für uns:** Der
  Concierge nutzt ausschließlich `chat/completions`
  (`OpenAiChatProvider`/Fireworks in `dl-ai/src/chat_provider.rs`),
  NICHT die Response API. Sollte je auf die Response API gewechselt
  werden, ist `store=False` Pflicht.
- Zertifizierungen: ISO 27001, ISO 27701 (Privacy), ISO 42001
  (AI-Management), SOC 2 Type II; Controls auf GDPR/CCPA gemappt.
  Zertifikats-PDFs im Trust Center abrufbar.

## Bewertung

Die No-Training-Zusage ist damit öffentlich dokumentiert und erfüllt den
Kern von §5.6. DeepSeek läuft als offenes Modell auf Fireworks-Infrastruktur
in den USA, es gilt derselbe Transfer-Rahmen wie bei OpenAI/Anthropic
(DPF/SCC dokumentieren).

## DPA-Nachweis (ERLEDIGT 2026-07-07)

- Kopie liegt hier: `fireworks-dpa-v3.2.pdf` (8 Seiten, Quelle:
  https://fireworks.ai/dpa — Direktlink zur jeweils aktuellen PDF).
- Das DPA ist laut eigenem Wortlaut Bestandteil des Agreements für jeden
  Business-Kunden der Services ("forms part of and is incorporated into
  the agreement"), eine separate Unterschrift ist nicht nötig.
- Enthält GDPR-Begriffe, Subprozessoren-Regelung und Standard Contractual
  Clauses (Transfer-Rahmen USA).
- Damit ist Launch-Gate 1 aus der Spec (§10) erfüllt.

Optionaler Beifang aus dem Trust Center (nicht Gate-relevant): Transfer
Impact Assessment, ISO-/SOC-2-Reports.
