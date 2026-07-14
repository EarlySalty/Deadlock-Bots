-- Backfill nach dem Reconcile-Fehl-Purge vom 2026-02-23: Diese Nutzer waren
-- nachweislich verifiziert und befreundet; ihr weiterhin vorhandener Rang belegt den Altlink.
-- Der Re-Friend feuert erst beim nächsten Voice-Join und schreibt niemanden ungefragt an.
UPDATE core.steam_links
SET unlink_reason = 'legacy_unlink'
WHERE verified = false
  AND is_steam_friend = false
  AND deadlock_rank IS NOT NULL
  AND unlink_reason IS NULL;
