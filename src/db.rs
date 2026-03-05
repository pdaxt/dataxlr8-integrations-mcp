use anyhow::Result;
use sqlx::PgPool;

pub async fn setup_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(
        r#"
        CREATE SCHEMA IF NOT EXISTS integrations;

        CREATE TABLE IF NOT EXISTS integrations.configs (
            id            TEXT PRIMARY KEY,
            platform      TEXT NOT NULL CHECK (platform IN ('linkedin', 'seek', 'indeed', 'xing')),
            credentials   JSONB NOT NULL DEFAULT '{}',
            field_mapping  JSONB NOT NULL DEFAULT '{}',
            active        BOOLEAN NOT NULL DEFAULT true,
            last_sync     TIMESTAMPTZ,
            created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
            UNIQUE (platform)
        );

        CREATE TABLE IF NOT EXISTS integrations.sync_log (
            id              TEXT PRIMARY KEY,
            config_id       TEXT NOT NULL REFERENCES integrations.configs(id) ON DELETE CASCADE,
            direction       TEXT NOT NULL CHECK (direction IN ('pull', 'push')),
            records_synced  INTEGER NOT NULL DEFAULT 0,
            errors          INTEGER NOT NULL DEFAULT 0,
            details         TEXT NOT NULL DEFAULT '',
            synced_at       TIMESTAMPTZ NOT NULL DEFAULT now()
        );

        CREATE INDEX IF NOT EXISTS idx_configs_platform ON integrations.configs(platform);
        CREATE INDEX IF NOT EXISTS idx_sync_log_config_id ON integrations.sync_log(config_id);
        CREATE INDEX IF NOT EXISTS idx_sync_log_synced_at ON integrations.sync_log(synced_at);
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
