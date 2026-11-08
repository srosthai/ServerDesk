use serde::{Deserialize, Serialize};

use crate::{core::AppError, storage::Database};

pub const DEFAULT_MONITORING_INTERVAL_SECONDS: u32 = 30;
pub const MIN_MONITORING_INTERVAL_SECONDS: u32 = 5;
pub const MAX_MONITORING_INTERVAL_SECONDS: u32 = 300;

const THEME_KEY: &str = "theme";
const MONITORING_INTERVAL_KEY: &str = "monitoring_interval_seconds";
const LAST_ACTIVE_SERVER_KEY: &str = "last_active_server_id";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPreferences {
    pub theme: ThemePreference,
    pub monitoring_interval_seconds: u32,
    pub last_active_server_id: Option<String>,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            monitoring_interval_seconds: DEFAULT_MONITORING_INTERVAL_SECONDS,
            last_active_server_id: None,
        }
    }
}

impl UserPreferences {
    pub fn load(database: &Database) -> Result<Self, AppError> {
        let theme = database
            .get_preference(THEME_KEY)?
            .and_then(|preference| ThemePreference::parse(&preference.value))
            .unwrap_or_default();

        let monitoring_interval_seconds = database
            .get_preference(MONITORING_INTERVAL_KEY)?
            .and_then(|preference| preference.value.parse::<u32>().ok())
            .filter(|seconds| is_valid_interval(*seconds))
            .unwrap_or(DEFAULT_MONITORING_INTERVAL_SECONDS);

        let last_active_server_id = database
            .get_preference(LAST_ACTIVE_SERVER_KEY)?
            .map(|preference| preference.value.trim().to_string())
            .filter(|value| !value.is_empty());

        Ok(Self {
            theme,
            monitoring_interval_seconds,
            last_active_server_id,
        })
    }

    pub fn save(&self, database: &Database) -> Result<Self, AppError> {
        self.validate()?;

        let mut normalized = self.clone();
        normalized.last_active_server_id = normalized
            .last_active_server_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        database.set_preference(THEME_KEY, normalized.theme.as_str())?;
        database.set_preference(
            MONITORING_INTERVAL_KEY,
            &normalized.monitoring_interval_seconds.to_string(),
        )?;

        match normalized.last_active_server_id.as_deref() {
            Some(server_id) => {
                database.set_preference(LAST_ACTIVE_SERVER_KEY, server_id)?;
            }
            None => {
                database.delete_preference(LAST_ACTIVE_SERVER_KEY)?;
            }
        }

        Ok(normalized)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if !is_valid_interval(self.monitoring_interval_seconds) {
            return Err(AppError::validation(format!(
                "Monitoring interval must be between {MIN_MONITORING_INTERVAL_SECONDS} and {MAX_MONITORING_INTERVAL_SECONDS} seconds."
            )));
        }

        Ok(())
    }
}

fn is_valid_interval(seconds: u32) -> bool {
    (MIN_MONITORING_INTERVAL_SECONDS..=MAX_MONITORING_INTERVAL_SECONDS).contains(&seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_database_is_empty() {
        let database = Database::in_memory().expect("database");
        let preferences = UserPreferences::load(&database).expect("load preferences");

        assert_eq!(preferences, UserPreferences::default());
        assert_eq!(preferences.theme, ThemePreference::System);
        assert_eq!(
            preferences.monitoring_interval_seconds,
            DEFAULT_MONITORING_INTERVAL_SECONDS
        );
        assert!(preferences.last_active_server_id.is_none());
    }

    #[test]
    fn saves_and_loads_preferences() {
        let database = Database::in_memory().expect("database");
        let preferences = UserPreferences {
            theme: ThemePreference::Dark,
            monitoring_interval_seconds: 60,
            last_active_server_id: Some("server-1".to_string()),
        };

        let saved = preferences.save(&database).expect("save preferences");
        assert_eq!(saved, preferences);
        assert_eq!(
            UserPreferences::load(&database).expect("load preferences"),
            preferences
        );
    }

    #[test]
    fn invalid_stored_values_fall_back_to_defaults() {
        let database = Database::in_memory().expect("database");
        database
            .set_preference(THEME_KEY, "sepia")
            .expect("set theme");
        database
            .set_preference(MONITORING_INTERVAL_KEY, "1")
            .expect("set interval");
        database
            .set_preference(LAST_ACTIVE_SERVER_KEY, "   ")
            .expect("set server");

        let preferences = UserPreferences::load(&database).expect("load preferences");
        assert_eq!(preferences.theme, ThemePreference::System);
        assert_eq!(
            preferences.monitoring_interval_seconds,
            DEFAULT_MONITORING_INTERVAL_SECONDS
        );
        assert!(preferences.last_active_server_id.is_none());
    }

    #[test]
    fn rejects_out_of_range_intervals() {
        let database = Database::in_memory().expect("database");

        for interval in [
            MIN_MONITORING_INTERVAL_SECONDS - 1,
            MAX_MONITORING_INTERVAL_SECONDS + 1,
        ] {
            let preferences = UserPreferences {
                monitoring_interval_seconds: interval,
                ..UserPreferences::default()
            };
            assert!(preferences.save(&database).is_err());
        }
    }

    #[test]
    fn clearing_last_server_removes_preference() {
        let database = Database::in_memory().expect("database");
        let preferences = UserPreferences {
            last_active_server_id: Some("server-1".to_string()),
            ..UserPreferences::default()
        };
        preferences.save(&database).expect("save");
        assert!(
            database
                .get_preference(LAST_ACTIVE_SERVER_KEY)
                .expect("get")
                .is_some()
        );

        let cleared = UserPreferences {
            last_active_server_id: None,
            ..preferences
        };
        cleared.save(&database).expect("clear");
        assert!(
            database
                .get_preference(LAST_ACTIVE_SERVER_KEY)
                .expect("get")
                .is_none()
        );
    }
}
