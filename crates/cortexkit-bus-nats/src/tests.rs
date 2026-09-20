use async_nats::{Event, ServerError};
use cortexkit_bus_trait::{BusError, ConnectionFailure};

use crate::error::{map_connect_error, permission_violation};
use crate::{ConnectConfig, ASYNC_NATS_VERSION_FLOOR};

#[test]
fn declared_async_nats_floor_is_not_below_the_specification() {
    let floor = ASYNC_NATS_VERSION_FLOOR
        .split('.')
        .map(|part| part.parse::<u32>().expect("numeric version component"))
        .collect::<Vec<_>>();
    assert!(floor[0] > 0 || floor[1] >= 33);
}

#[test]
fn inbox_prefix_is_credential_scoped() {
    let config = ConnectConfig::new("UABC234").expect("valid public nkey");
    assert_eq!(config.inbox_prefix(), "_INBOX.UABC234");
    assert!(ConnectConfig::new("UABC.bad").is_err());
}

#[test]
fn connection_boundary_taxonomy_matches_kick_drop_and_severed_write() {
    let refused = map_connect_error("Authorization Violation");
    assert!(matches!(refused, BusError::Denied { .. }));

    let dropped = map_connect_error("connection reset by peer");
    assert!(matches!(dropped, BusError::Unavailable { .. }));

    let severed =
        cortexkit_bus_trait::map_connection_failure(ConnectionFailure::WriteOnSeveredSocket);
    assert!(matches!(severed, BusError::Unavailable { .. }));

    let kicked_then_refused =
        permission_violation(&Event::ServerError(ServerError::AuthorizationViolation))
            .expect("authorization refusal is observable")
            .as_denied();
    assert!(matches!(kicked_then_refused, BusError::Denied { .. }));
}
