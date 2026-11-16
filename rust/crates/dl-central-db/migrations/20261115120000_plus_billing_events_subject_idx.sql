SET LOCAL lock_timeout TO '5s';

CREATE INDEX IF NOT EXISTS plus_billing_events_object_id_idx
    ON steam.plus_billing_events (object_id);

CREATE INDEX IF NOT EXISTS plus_billing_events_subscription_idx
    ON steam.plus_billing_events USING hash ((payload #>> '{data,object,subscription}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_subscription_object_idx
    ON steam.plus_billing_events USING hash ((payload #>> '{data,object,subscription,id}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_invoice_subscription_idx
    ON steam.plus_billing_events USING hash ((payload #>> '{data,object,parent,subscription_details,subscription}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_payment_intent_idx
    ON steam.plus_billing_events USING hash ((payload #>> '{data,object,payment_intent}'));

CREATE INDEX IF NOT EXISTS plus_billing_events_payment_intent_object_idx
    ON steam.plus_billing_events USING hash ((payload #>> '{data,object,payment_intent,id}'));

COMMENT ON TABLE steam.plus_billing_events IS
    'Stripe webhook log of the Steam bot. stripe_event_id is the idempotency key against re-delivery. Since 20261115120000 the table is also read: the Steam bot (steam-persistence tip_event_payloads and subscription_log) looks up earlier events of one payment intent or subscription. A lookup uses an index only when it compares object_id or exactly one of the indexed payload #>> paths with =; a wrapped or ->/->> spelled expression scans the whole table. The payload paths use hash indexes because expanded Stripe objects can exceed the btree row size.';

COMMENT ON INDEX steam.plus_billing_events_object_id_idx IS
    'Lookup by object_id = $1.';

COMMENT ON INDEX steam.plus_billing_events_subscription_idx IS
    'Lookup by payload #>> ''{data,object,subscription}'' = $1.';

COMMENT ON INDEX steam.plus_billing_events_subscription_object_idx IS
    'Lookup by payload #>> ''{data,object,subscription,id}'' = $1.';

COMMENT ON INDEX steam.plus_billing_events_invoice_subscription_idx IS
    'Lookup by payload #>> ''{data,object,parent,subscription_details,subscription}'' = $1.';

COMMENT ON INDEX steam.plus_billing_events_payment_intent_idx IS
    'Lookup by payload #>> ''{data,object,payment_intent}'' = $1.';

COMMENT ON INDEX steam.plus_billing_events_payment_intent_object_idx IS
    'Lookup by payload #>> ''{data,object,payment_intent,id}'' = $1.';
