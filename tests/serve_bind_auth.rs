//! Serve bind/auth policy contract (Phase 0).
//!
//! Non-loopback requires --require-auth + token; --require-auth does not imply loopback.

use litecode::serve::validate_serve_bind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn all_interfaces(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
}

#[test]
fn bind_auth_matrix() {
    // A: loopback + require-auth + token
    assert!(validate_serve_bind(loopback(7483), false, true, true).is_ok());
    // B: 0.0.0.0 + require-auth + token
    assert!(validate_serve_bind(all_interfaces(7483), false, true, true).is_ok());
    // C: 0.0.0.0 without auth
    assert!(validate_serve_bind(all_interfaces(7483), false, false, false).is_err());
    // D: require-auth without token
    assert!(validate_serve_bind(loopback(7483), false, true, false).is_err());
}
