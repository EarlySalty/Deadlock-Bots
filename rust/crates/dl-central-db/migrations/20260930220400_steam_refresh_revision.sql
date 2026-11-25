-- Getrennte Revision für Refresh-Zugänge. Änderungen einer Gerätefreigabe
-- dürfen einen gleichzeitig vorbereiteten Refresh nicht fälschlich veralten.
ALTER TABLE steam.account_credentials
    ADD COLUMN refresh_revision BIGINT NOT NULL DEFAULT 0
    CHECK (refresh_revision >= 0);
