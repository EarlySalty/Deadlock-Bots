DROP INDEX steam.web_api_reservations_completed_window_idx;

ALTER TABLE steam.web_api_budget DROP COLUMN last_pruned_at;
