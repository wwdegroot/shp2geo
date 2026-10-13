use std::sync::Arc;

const DEFAULT_PORT: u16 = 3000;
static DEFAULT_HOSTNAME: &str = "127.0.0.1";
const DEFAULT_MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024; // 25 MB
const DEFAULT_MAX_EXTRACTED_BYTES: usize = 1024 * 1024 * 1024; // 1 GB

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum accepted upload (zip) size in bytes.
    pub max_upload_bytes: usize,
    /// Maximum total decompressed size of all zip entries in bytes.
    pub max_extracted_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            max_extracted_bytes: DEFAULT_MAX_EXTRACTED_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub port: u16,
    pub hostname: String,
    pub limits: Arc<Limits>,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let port = env_parse("APP_PORT").unwrap_or(DEFAULT_PORT);
        let hostname = env_parse("APP_HOSTNAME").unwrap_or(DEFAULT_HOSTNAME.to_string());
        let mut limits_cfg = Limits::default();
        if let Some(v) = env_parse("MAX_UPLOAD_BYTES") {
            limits_cfg.max_upload_bytes = v;
        }
        if let Some(v) = env_parse("MAX_EXTRACTED_BYTES") {
            limits_cfg.max_extracted_bytes = v;
        }
        let limits = Arc::new(limits_cfg);
        AppConfig {
            port,
            hostname,
            limits,
        }
    }
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<T>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let l = Limits::default();
        assert_eq!(l.max_upload_bytes, 25 * 1024 * 1024);
        assert_eq!(l.max_extracted_bytes, 1024 * 1024 * 1024);
    }
}
