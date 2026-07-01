#![allow(dead_code)]

use std::{
    env,
    error::Error,
    fmt::{self, Display},
    net::{IpAddr, SocketAddr},
};

#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub object_storage: ObjectStorageConfig,
    pub providers: ProviderConfig,
    pub scheduler: SchedulerConfig,
    pub admin: AdminConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
}

impl ServerConfig {
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct ObjectStorageConfig {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub prefix: String,
    pub endpoint: String,
    pub region: String,
    pub force_path_style: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub gemini_api_key: String,
    pub meshy_api_key: String,
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub time: String,
    pub timezone: String,
}

#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    MissingVars(Vec<&'static str>),
    InvalidVar {
        name: &'static str,
        value: String,
        reason: &'static str,
    },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut missing = Vec::new();

        let database_url = required_var("DATABASE_URL", &mut missing);
        let object_storage_access_key_id =
            required_var("OBJECT_STORAGE_ACCESS_KEY_ID", &mut missing);
        let object_storage_secret_access_key =
            required_var("OBJECT_STORAGE_SECRET_ACCESS_KEY", &mut missing);
        let object_storage_bucket = required_var("OBJECT_STORAGE_BUCKET", &mut missing);
        let object_storage_prefix = required_var("OBJECT_STORAGE_PREFIX", &mut missing);
        let object_storage_endpoint = required_var("OBJECT_STORAGE_ENDPOINT", &mut missing);
        let object_storage_region = required_var("OBJECT_STORAGE_REGION", &mut missing);
        let object_storage_force_path_style =
            required_var("OBJECT_STORAGE_FORCE_PATH_STYLE", &mut missing);
        let gemini_api_key = required_var("GEMINI_API_KEY", &mut missing);
        let meshy_api_key = required_var("MESHY_API_KEY", &mut missing);
        let schedule_time = required_var("SCHEDULE_TIME", &mut missing);
        let schedule_timezone = required_var("SCHEDULE_TIMEZONE", &mut missing);
        let admin_api_key = required_var("ADMIN_API_KEY", &mut missing);

        if !missing.is_empty() {
            return Err(ConfigError::MissingVars(missing));
        }

        let (
            database_url,
            object_storage_access_key_id,
            object_storage_secret_access_key,
            object_storage_bucket,
            object_storage_prefix,
            object_storage_endpoint,
            object_storage_region,
            object_storage_force_path_style,
            gemini_api_key,
            meshy_api_key,
            schedule_time,
            schedule_timezone,
            admin_api_key,
        ) = match (
            database_url,
            object_storage_access_key_id,
            object_storage_secret_access_key,
            object_storage_bucket,
            object_storage_prefix,
            object_storage_endpoint,
            object_storage_region,
            object_storage_force_path_style,
            gemini_api_key,
            meshy_api_key,
            schedule_time,
            schedule_timezone,
            admin_api_key,
        ) {
            (
                Some(database_url),
                Some(object_storage_access_key_id),
                Some(object_storage_secret_access_key),
                Some(object_storage_bucket),
                Some(object_storage_prefix),
                Some(object_storage_endpoint),
                Some(object_storage_region),
                Some(object_storage_force_path_style),
                Some(gemini_api_key),
                Some(meshy_api_key),
                Some(schedule_time),
                Some(schedule_timezone),
                Some(admin_api_key),
            ) => (
                database_url,
                object_storage_access_key_id,
                object_storage_secret_access_key,
                object_storage_bucket,
                object_storage_prefix,
                object_storage_endpoint,
                object_storage_region,
                object_storage_force_path_style,
                gemini_api_key,
                meshy_api_key,
                schedule_time,
                schedule_timezone,
                admin_api_key,
            ),
            _ => return Err(ConfigError::MissingVars(Vec::new())),
        };

        let server = ServerConfig {
            host: optional_var("HOST")
                .map(|host| parse_ip_addr("HOST", &host))
                .transpose()?
                .unwrap_or(IpAddr::from([0, 0, 0, 0])),
            port: optional_var("PORT")
                .map(|port| parse_port("PORT", &port))
                .transpose()?
                .unwrap_or(8080),
        };

        if !object_storage_prefix.ends_with('/') {
            return Err(ConfigError::InvalidVar {
                name: "OBJECT_STORAGE_PREFIX",
                value: object_storage_prefix,
                reason: "must end with a slash",
            });
        }

        let force_path_style = parse_bool(
            "OBJECT_STORAGE_FORCE_PATH_STYLE",
            &object_storage_force_path_style,
        )?;
        validate_schedule_time("SCHEDULE_TIME", &schedule_time)?;

        Ok(Self {
            server,
            database: DatabaseConfig { url: database_url },
            object_storage: ObjectStorageConfig {
                access_key_id: object_storage_access_key_id,
                secret_access_key: object_storage_secret_access_key,
                bucket: object_storage_bucket,
                prefix: object_storage_prefix,
                endpoint: object_storage_endpoint,
                region: object_storage_region,
                force_path_style,
            },
            providers: ProviderConfig {
                gemini_api_key,
                meshy_api_key,
            },
            scheduler: SchedulerConfig {
                time: schedule_time,
                timezone: schedule_timezone,
            },
            admin: AdminConfig {
                api_key: admin_api_key,
            },
        })
    }
}

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVars(names) => write!(
                formatter,
                "missing required environment variable(s): {}",
                names.join(", ")
            ),
            Self::InvalidVar {
                name,
                value,
                reason,
            } => {
                write!(
                    formatter,
                    "invalid environment variable {name}={value:?}: {reason}"
                )
            }
        }
    }
}

impl Error for ConfigError {}

fn required_var(name: &'static str, missing: &mut Vec<&'static str>) -> Option<String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            missing.push(name);
            None
        }
    }
}

fn optional_var(name: &'static str) -> Option<String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

fn parse_ip_addr(name: &'static str, value: &str) -> Result<IpAddr, ConfigError> {
    value.parse().map_err(|_| ConfigError::InvalidVar {
        name,
        value: value.to_owned(),
        reason: "must be a valid IP address",
    })
}

fn parse_port(name: &'static str, value: &str) -> Result<u16, ConfigError> {
    value.parse().map_err(|_| ConfigError::InvalidVar {
        name,
        value: value.to_owned(),
        reason: "must be a valid TCP port",
    })
}

fn parse_bool(name: &'static str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::InvalidVar {
            name,
            value: value.to_owned(),
            reason: "must be true or false",
        }),
    }
}

fn validate_schedule_time(name: &'static str, value: &str) -> Result<(), ConfigError> {
    let mut parts = value.split(':');
    let hour = parts.next();
    let minute = parts.next();

    if parts.next().is_some() {
        return Err(invalid_schedule_time(name, value));
    }

    match (hour, minute) {
        (Some(hour), Some(minute)) if hour.len() == 2 && minute.len() == 2 => {
            let hour = parse_time_part(name, value, hour)?;
            let minute = parse_time_part(name, value, minute)?;

            if hour <= 23 && minute <= 59 {
                Ok(())
            } else {
                Err(invalid_schedule_time(name, value))
            }
        }
        _ => Err(invalid_schedule_time(name, value)),
    }
}

fn parse_time_part(name: &'static str, full_value: &str, value: &str) -> Result<u8, ConfigError> {
    value
        .parse()
        .map_err(|_| invalid_schedule_time(name, full_value))
}

fn invalid_schedule_time(name: &'static str, value: &str) -> ConfigError {
    ConfigError::InvalidVar {
        name,
        value: value.to_owned(),
        reason: "must use HH:MM in 24-hour time",
    }
}
