SET LOCAL lock_timeout TO '5s';

CREATE INDEX IF NOT EXISTS plus_billing_events_object_id_idx
    ON steam.plus_billing_events (object_id);

CREATE INDEX IF NOT EXISTS plus_billing_events_subscription_idx
    ON steam.plus_billing_events ((payload #>> '{data,object,subscription}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_subscription_object_idx
    ON steam.plus_billing_events ((payload #>> '{data,object,subscription,id}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_invoice_subscription_idx
    ON steam.plus_billing_events ((payload #>> '{data,object,parent,subscription_details,subscription}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_payment_intent_idx
    ON steam.plus_billing_events ((payload #>> '{data,object,payment_intent}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_payment_intent_object_idx
    ON steam.plus_billing_events ((payload #>> '{data,object,payment_intent,id}'));
