SET lock_timeout TO '5s';

UPDATE core.privacy_field_registry
   SET erasure_action = 'delete_row_on_user_delete',
       reason = CASE column_name
           WHEN 'discord_id' THEN 'Discord user id of a supporter; the user deletion path removes every source row of the member'
           ELSE 'Id of the proof inside the source (subscription id, Twitch channel); removed with the row on user deletion'
       END
 WHERE schema_name = 'core'
   AND table_name = 'supporter_sources'
   AND column_name IN ('discord_id', 'source_ref');
