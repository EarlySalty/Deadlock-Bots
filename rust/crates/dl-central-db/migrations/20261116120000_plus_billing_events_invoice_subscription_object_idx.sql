SET LOCAL lock_timeout TO '5s';

CREATE INDEX IF NOT EXISTS plus_billing_events_invoice_subscription_object_idx
    ON steam.plus_billing_events USING hash ((payload #>> '{data,object,parent,subscription_details,subscription,id}'));

COMMENT ON INDEX steam.plus_billing_events_invoice_subscription_object_idx IS
    'Lookup by payload #>> ''{data,object,parent,subscription_details,subscription,id}'' = $1. Fifth exact arm of the Steam bot subscription_log lookup; without it the OR of all arms cannot use a BitmapOr and falls back to a full scan.';
