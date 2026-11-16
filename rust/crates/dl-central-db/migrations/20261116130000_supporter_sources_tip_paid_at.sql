SET LOCAL lock_timeout TO '5s';

ALTER TABLE core.supporter_sources
    ADD COLUMN IF NOT EXISTS paid_at TIMESTAMPTZ;

ALTER TABLE core.supporter_sources
    DROP CONSTRAINT IF EXISTS supporter_sources_paid_at_tip_only;

CREATE OR REPLACE FUNCTION core.supporter_sources_tip_paid_at() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NEW.source = 'stripe'
       AND starts_with(NEW.source_ref, 'tip:')
       AND NEW.paid_at IS NULL THEN
        IF TG_OP = 'UPDATE' AND OLD.paid_at IS NOT NULL THEN
            NEW.paid_at := OLD.paid_at;
        ELSE
            NEW.paid_at := NEW.updated_at;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

REVOKE ALL ON FUNCTION core.supporter_sources_tip_paid_at() FROM PUBLIC;

DROP TRIGGER IF EXISTS supporter_sources_tip_paid_at ON core.supporter_sources;
CREATE TRIGGER supporter_sources_tip_paid_at
    BEFORE INSERT OR UPDATE ON core.supporter_sources
    FOR EACH ROW EXECUTE FUNCTION core.supporter_sources_tip_paid_at();

ALTER TABLE core.supporter_sources DISABLE TRIGGER supporter_sources_privacy_gate;

UPDATE core.supporter_sources
   SET paid_at = updated_at
 WHERE source = 'stripe'
   AND starts_with(source_ref, 'tip:')
   AND paid_at IS NULL;

ALTER TABLE core.supporter_sources ENABLE TRIGGER supporter_sources_privacy_gate;

ALTER TABLE core.supporter_sources
    ADD CONSTRAINT supporter_sources_paid_at_tip_only CHECK (
        (source = 'stripe' AND starts_with(source_ref, 'tip:')) = (paid_at IS NOT NULL)
        AND (paid_at IS NULL OR isfinite(paid_at))
    );

COMMENT ON COLUMN core.supporter_sources.paid_at IS
    'Payment time of a one-off tip: present and finite on every row with source stripe and source_ref tip:<payment intent> (TIP_REF_PREFIX in dl_core::runtime_config::supporter_wire), NULL on every other row. A tip counts for the funding goal in the Europe/Berlin calendar month of paid_at and grants the supporter role for TIP_VALID_DAYS days (dl_core::runtime_config::supporter_wire), so its writer sets valid_until = paid_at + TIP_VALID_DAYS days; the privacy gate may shorten valid_until later. A writer that leaves paid_at NULL gets it from the trigger supporter_sources_tip_paid_at: the earlier paid_at on an update, else updated_at, which the Steam bot sets to the payment time. Tip rows written before 20261116130000 were backfilled from updated_at with the privacy gate disabled inside the migration, so no row was skipped.';

COMMENT ON CONSTRAINT supporter_sources_paid_at_tip_only ON core.supporter_sources IS
    'paid_at is present exactly on Stripe tip rows (source stripe, source_ref starting with tip:) and is finite.';

COMMENT ON FUNCTION core.supporter_sources_tip_paid_at() IS
    'Fills paid_at on a Stripe tip row written without it: an update keeps the earlier paid_at, otherwise paid_at becomes updated_at. Runs after supporter_sources_privacy_gate (BEFORE triggers fire in name order), so a write the gate drops never reaches it.';
