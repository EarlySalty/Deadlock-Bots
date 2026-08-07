-- Plus-Tier des Steam-Bots: Stripe-Abozustand und Webhook-Dedup.
-- Zwei getrennte Tabellen mit Absicht: der Abozustand ist das Entitlement
-- (daraus leitet der Reconciler die Discord-Rolle ab), die Event-Tabelle ist
-- reine Idempotenz-Schranke gegen Stripe-Re-Delivery und wird nie fachlich gelesen.
--
-- Zeitstempel sind timestamptz und Flags sind boolean. Der Twitch-Bot fuehrt
-- dieselben Daten als TEXT/INTEGER (SQLite-Erbe seines Python-Vorgaengers);
-- das wird hier bewusst nicht uebernommen.

CREATE TABLE IF NOT EXISTS steam.plus_subscriptions (
    stripe_subscription_id TEXT PRIMARY KEY,
    discord_id BIGINT NOT NULL,
    stripe_customer_id TEXT,
    -- Stripe-Status wird durchgereicht (active, trialing, past_due, canceled, ...).
    -- Kein Enum: neue Stripe-Status duerfen den Webhook nicht zum Fehler bringen.
    status TEXT NOT NULL DEFAULT 'unknown',
    current_period_start TIMESTAMPTZ,
    current_period_end TIMESTAMPTZ,
    cancel_at_period_end BOOLEAN NOT NULL DEFAULT FALSE,
    canceled_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ,
    -- Letztes angewandtes Stripe-Event, nur fuer Diagnose.
    last_event_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Hotpath des Reconcilers: alle Abos eines Nutzers (ein Nutzer kann nacheinander
-- mehrere Abos haben, etwa nach Kuendigung und Neuabschluss).
CREATE INDEX IF NOT EXISTS plus_subscriptions_discord_id_idx
    ON steam.plus_subscriptions (discord_id);

-- Sollmenge des Reconcilers: wer hat gerade Anspruch auf die Rolle.
CREATE INDEX IF NOT EXISTS plus_subscriptions_active_idx
    ON steam.plus_subscriptions (discord_id)
    WHERE status IN ('active', 'trialing') AND ended_at IS NULL;

-- Nachzieh-Pfad gegen verlorene Webhooks: Abos, deren Zustand alt ist, werden
-- per GET /v1/subscriptions/{id} nachgeholt.
CREATE INDEX IF NOT EXISTS plus_subscriptions_updated_at_idx
    ON steam.plus_subscriptions (updated_at);

CREATE TABLE IF NOT EXISTS steam.plus_billing_events (
    stripe_event_id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    object_id TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    livemode BOOLEAN NOT NULL DEFAULT FALSE,
    payload JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS plus_billing_events_received_at_idx
    ON steam.plus_billing_events (received_at);
