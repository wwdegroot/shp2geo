//! Runtime configuration and resource limits.

const DEFAULT_PORT: u16 = 3000;
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

impl Limits {
    pub fn from_env() -> Self {
        let mut limits = Limits::default();
        if let Some(v) = env_parse("MAX_UPLOAD_BYTES") {
            limits.max_upload_bytes = v;
        }
        if let Some(v) = env_parse("MAX_EXTRACTED_BYTES") {
            limits.max_extracted_bytes = v;
        }
        limits
    }
}

pub fn port_from_env() -> u16 {
    env_parse("PORT").unwrap_or(DEFAULT_PORT)
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
        assert_eq!(port_from_env(), 3000);
    }
}
