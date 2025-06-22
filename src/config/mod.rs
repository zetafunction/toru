use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub struct Config {
    pub api_keys: ApiKeys,
}

#[derive(Deserialize)]
pub struct ApiKeys {
    pub omdb: Option<String>,
}

pub fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();

    CONFIG.get_or_init(|| {
        toml::from_str::<Config>(
            &std::fs::read_to_string("config.toml").expect("failed to read config.toml"),
        )
        .expect("config.toml contains invalid toml")
    })
}
