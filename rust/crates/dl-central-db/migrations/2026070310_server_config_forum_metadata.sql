ALTER TABLE server_config.desired_channels
    ADD COLUMN IF NOT EXISTS default_auto_archive_duration INTEGER;

ALTER TABLE server_config.live_snapshot_channels
    ADD COLUMN IF NOT EXISTS default_auto_archive_duration INTEGER;
