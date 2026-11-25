ALTER TABLE bot.concierge_pate_requests
    DROP CONSTRAINT concierge_pate_requests_status_check;

ALTER TABLE bot.concierge_pate_requests
    ADD CONSTRAINT concierge_pate_requests_status_check
    CHECK (status IN ('open', 'claimed', 'closed_unbesetzt', 'closed_opted_out'));
