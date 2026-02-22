use std::net::SocketAddr;

use crate::infrastructure::config::{ConfigError, ProductionConfig};

#[test]
fn config_requires_database_url() {
    let cfg = ProductionConfig::from_lookup(|key| match key {
        "PROD_API_ADDR" => Some("127.0.0.1:4100".to_string()),
        _ => None,
    });

    assert!(matches!(
        cfg,
        Err(ConfigError::MissingRequired {
            key: "PROD_API_DATABASE_URL"
        })
    ));
}

#[test]
fn config_uses_defaults_when_optional_values_absent() {
    let cfg = ProductionConfig::from_lookup(|key| {
        if key == "PROD_API_DATABASE_URL" {
            Some("postgres://127.0.0.1:55432/openportio".to_string())
        } else {
            None
        }
    })
    .expect("config should parse");

    assert_eq!(cfg.addr, SocketAddr::from(([127, 0, 0, 1], 4100)));
    assert_eq!(cfg.service_name, "production-api");
    assert_eq!(cfg.max_db_connections, 10);
    assert!(cfg.run_migrations);
    assert_eq!(cfg.migration_retry_seconds, 5);
    assert!(!cfg.enable_drill_routes);
}
