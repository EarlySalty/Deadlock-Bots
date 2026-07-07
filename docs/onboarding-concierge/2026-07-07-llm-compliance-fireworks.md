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

## Restpunkt (Owner)

Aus dem Trust Center das DPA-Dokument beziehen und eine Kopie ablegen
(dieser Ordner oder Ablage des Owners). Danach ist Launch-Gate 1 aus der
Spec (§10) erfüllt und der Testmodus kann für echte neue Joins geöffnet
werden.
