SET LOCAL lock_timeout TO '5s';

ALTER TABLE core.supporter_sources
    ADD COLUMN IF NOT EXISTS paid_at TIMESTAMPTZ CONSTRAINT supporter_sources_paid_at_tip_only CHECK (
        paid_at IS NULL
        OR (isfinite(paid_at) AND source = 'stripe' AND starts_with(source_ref, 'tip:'))
    );

UPDATE core.supporter_sources
   SET paid_at = updated_at
 WHERE source = 'stripe'
   AND starts_with(source_ref, 'tip:')
   AND paid_at IS NULL;

COMMENT ON COLUMN core.supporter_sources.paid_at IS
    'Payment time of a one-off tip, set only on rows with source stripe and source_ref tip:<payment intent>, NULL on every other row. A tip counts for the funding goal in the Europe/Berlin calendar month of paid_at and grants the supporter role for 31 days (TIP_VALID_DAYS in the Steam bot), so its writer sets valid_until = paid_at + 31 days. Tip rows written before 20261116130000 were backfilled from updated_at, which the Steam bot had set to the payment time. Finite.';

COMMENT ON CONSTRAINT supporter_sources_paid_at_tip_only ON core.supporter_sources IS
    'paid_at is finite and only present on Stripe tip rows (source stripe, source_ref starting with tip:).';
