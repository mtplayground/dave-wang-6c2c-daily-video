use std::{
    error::Error,
    fmt::{self, Display},
    str::FromStr,
    time::Duration,
};

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tracing::info;

use crate::config::DatabaseConfig;

#[derive(Debug)]
pub enum DbError {
    InvalidDatabaseUrl(sqlx::Error),
    Connect(sqlx::Error),
    Migrate(sqlx::migrate::MigrateError),
}

pub async fn connect_and_migrate(config: &DatabaseConfig) -> Result<PgPool, DbError> {
    let pool = connect(config).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, DbError> {
    let options = PgConnectOptions::from_str(&config.url)
        .map_err(DbError::InvalidDatabaseUrl)?
        .statement_cache_capacity(0);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(DbError::Connect)?;

    info!("connected to PostgreSQL");
    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), DbError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(DbError::Migrate)?;

    info!("database migrations applied");
    Ok(())
}

impl Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDatabaseUrl(_) => write!(formatter, "invalid DATABASE_URL"),
            Self::Connect(_) => write!(formatter, "failed to connect to PostgreSQL"),
            Self::Migrate(_) => write!(formatter, "failed to run database migrations"),
        }
    }
}

impl Error for DbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidDatabaseUrl(err) => Some(err),
            Self::Connect(err) => Some(err),
            Self::Migrate(err) => Some(err),
        }
    }
}
