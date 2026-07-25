SET lock_timeout TO '2s';

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM scrim.matches AS m
          JOIN scrim.match_result_refs AS r ON r.steam_match_id = m.steam_match_id
         WHERE m.steam_match_id IS NOT NULL
           AND r.match_id <> m.id
    ) THEN
        RAISE EXCEPTION 'scrim legacy steam_match_id conflicts with an existing result ref for another match'
            USING ERRCODE = '23505';
    END IF;
END;
$$;

WITH eligible_matches AS (
    SELECT m.id,
           m.steam_match_id,
           m.winner_team_id,
           m.result_json,
           COALESCE(m.updated_at, now()) AS result_at
      FROM scrim.matches AS m
     WHERE m.steam_match_id IS NOT NULL
       AND m.winner_team_id IS NOT NULL
       AND m.result_json IS NOT NULL
       AND (
            lower(COALESCE(m.status, '')) IN ('finished', 'completed')
            OR lower(COALESCE(m.lobby_state, '')) = 'finished'
       )
)
UPDATE scrim.match_result_refs AS r
   SET fetch_status = CASE WHEN r.fetch_status = 'fetched' THEN r.fetch_status ELSE 'fetched' END,
       fetched_at = COALESCE(r.fetched_at, eligible.result_at),
       winner_team_id = COALESCE(r.winner_team_id, eligible.winner_team_id),
       raw_result_json = CASE
           WHEN r.raw_result_json IS NULL OR r.raw_result_json = '{}'::jsonb THEN eligible.result_json
           ELSE r.raw_result_json
       END,
       normalized_result_json = CASE
           WHEN r.normalized_result_json IS NULL OR r.normalized_result_json = '{}'::jsonb THEN eligible.result_json
           ELSE r.normalized_result_json
       END,
       validation_status = 'valid',
       updated_at = now()
   FROM eligible_matches AS eligible
  WHERE r.match_id = eligible.id
    AND r.steam_match_id = eligible.steam_match_id
    AND r.validation_status = 'unvalidated'
    AND r.voided_at IS NULL
    AND r.superseded_by_ref_id IS NULL;

WITH eligible_matches AS (
    SELECT m.id,
           m.steam_match_id,
           m.winner_team_id,
           m.result_json,
           COALESCE(m.updated_at, now()) AS result_at
      FROM scrim.matches AS m
     WHERE m.steam_match_id IS NOT NULL
       AND m.winner_team_id IS NOT NULL
       AND m.result_json IS NOT NULL
       AND (
            lower(COALESCE(m.status, '')) IN ('finished', 'completed')
            OR lower(COALESCE(m.lobby_state, '')) = 'finished'
       )
)
INSERT INTO scrim.match_result_refs(
    match_id,
    steam_match_id,
    source_user_id,
    source_display_name,
    fetch_status,
    fetched_at,
    winner_team_id,
    raw_result_json,
    normalized_result_json,
    validation_status
)
SELECT
    eligible.id,
    eligible.steam_match_id,
    '900000000000001',
    'Migration',
    'fetched',
    eligible.result_at,
    eligible.winner_team_id,
    eligible.result_json,
    eligible.result_json,
    'valid'
  FROM eligible_matches AS eligible
 WHERE NOT EXISTS (
        SELECT 1
          FROM scrim.match_result_refs AS existing
         WHERE existing.match_id = eligible.id
           AND existing.steam_match_id = eligible.steam_match_id
   )
;

WITH deterministic_complete_refs AS (
    SELECT DISTINCT ON (r.match_id)
           r.match_id,
           r.id AS result_ref_id,
           COALESCE(r.fetched_at, r.updated_at, r.entered_at) AS selected_at
      FROM scrim.match_result_refs AS r
     WHERE r.validation_status = 'valid'
       AND r.fetch_status = 'fetched'
       AND r.steam_match_id IS NOT NULL
       AND r.winner_team_id IS NOT NULL
       AND r.voided_at IS NULL
       AND r.superseded_by_ref_id IS NULL
     ORDER BY r.match_id,
              r.fetched_at DESC NULLS LAST,
              r.updated_at DESC,
              r.entered_at DESC,
              r.steam_match_id DESC,
              r.id DESC
)
INSERT INTO scrim.match_result_selections(
    match_id,
    result_ref_id,
    selected_at,
    selected_by_user_id,
    selected_by_display_name,
    selection_reason
)
SELECT match_id,
       result_ref_id,
       selected_at,
       '900000000000001',
       'Migration',
       'legacy_match_backfill'
  FROM deterministic_complete_refs
ON CONFLICT (match_id) DO NOTHING;

CREATE OR REPLACE VIEW scrim.selected_match_results AS
SELECT
    m.id AS match_id,
    selected_ref.id AS result_ref_id,
    selected_ref.steam_match_id,
    selected_ref.winner_team_id,
    selected_ref.normalized_result_json AS result_json,
    CASE WHEN selected_ref.id IS NOT NULL THEN 'result_ref' ELSE NULL END AS source,
    CASE WHEN selected_ref.id IS NOT NULL THEN selection.selected_at ELSE NULL END AS selected_at,
    CASE WHEN selected_ref.id IS NOT NULL THEN selection.selected_by_user_id ELSE NULL END AS selected_by_user_id
FROM scrim.matches AS m
LEFT JOIN scrim.match_result_selections AS selection
       ON selection.match_id = m.id
LEFT JOIN scrim.match_result_refs AS selected_ref
       ON selected_ref.id = selection.result_ref_id
      AND selected_ref.fetch_status = 'fetched'
      AND selected_ref.validation_status = 'valid'
      AND selected_ref.winner_team_id IS NOT NULL
      AND selected_ref.voided_at IS NULL
      AND selected_ref.superseded_by_ref_id IS NULL;

RESET lock_timeout;
