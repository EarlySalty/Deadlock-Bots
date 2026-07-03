CREATE SEQUENCE IF NOT EXISTS voice.lfg_posts_id_seq;

ALTER TABLE voice.lfg_posts
    ADD COLUMN IF NOT EXISTS id BIGINT;

ALTER SEQUENCE voice.lfg_posts_id_seq
    OWNED BY voice.lfg_posts.id;

UPDATE voice.lfg_posts
   SET id = nextval('voice.lfg_posts_id_seq')
 WHERE id IS NULL;

ALTER TABLE voice.lfg_posts
    ALTER COLUMN id SET DEFAULT nextval('voice.lfg_posts_id_seq');

ALTER TABLE voice.lfg_posts
    ALTER COLUMN id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conrelid = 'voice.lfg_posts'::regclass
           AND conname = 'lfg_posts_pkey'
    ) THEN
        ALTER TABLE voice.lfg_posts
            ADD CONSTRAINT lfg_posts_pkey PRIMARY KEY (id);
    END IF;
END $$;

SELECT setval(
    'voice.lfg_posts_id_seq',
    GREATEST(COALESCE((SELECT max(id) FROM voice.lfg_posts), 0), 1),
    true
);
