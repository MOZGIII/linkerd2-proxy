mod api;
mod authorize;
mod config;
pub mod defaults;
mod store;
#[cfg(test)]
mod tests;

pub use self::authorize::{NewAuthorizeHttp, NewAuthorizeTcp};
pub use self::config::Config;
pub(crate) use self::store::Store;

pub use linkerd_app_core::metrics::{AuthzLabels, ServerLabel};
use linkerd_app_core::{
    tls,
    transport::{ClientAddr, OrigDstAddr, Remote},
    Result,
};
use linkerd_cache::Cached;
pub use linkerd_server_policy::{
    self as policy, Authentication, Authorization, Protocol, ServerPolicy, Suffix,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::watch;

#[derive(Clone, Debug, Error)]
#[error("unauthorized connection on {kind}/{name}")]
pub struct DeniedUnauthorized {
    kind: std::sync::Arc<str>,
    name: std::sync::Arc<str>,
}

pub trait GetPolicy {
    // Returns the traffic policy configured for the destination address.
    fn get_policy(&self, dst: OrigDstAddr) -> AllowPolicy;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultPolicy {
    Allow(ServerPolicy),
    Deny,
}

#[derive(Clone, Debug)]
pub struct AllowPolicy {
    dst: OrigDstAddr,
    server: Cached<watch::Receiver<ServerPolicy>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Permit {
    pub dst: OrigDstAddr,
    pub protocol: Protocol,

    pub labels: AuthzLabels,
}

// === impl DefaultPolicy ===

impl From<ServerPolicy> for DefaultPolicy {
    fn from(p: ServerPolicy) -> Self {
        DefaultPolicy::Allow(p)
    }
}

impl From<DefaultPolicy> for ServerPolicy {
    fn from(d: DefaultPolicy) -> Self {
        match d {
            DefaultPolicy::Allow(p) => p,
            DefaultPolicy::Deny => ServerPolicy {
                protocol: Protocol::Opaque,
                authorizations: vec![].into(),
                kind: "default".into(),
                name: "deny".into(),
            },
        }
    }
}

// === impl AllowPolicy ===

impl AllowPolicy {
    #[cfg(any(test, fuzzing))]
    pub(crate) fn for_test(
        dst: OrigDstAddr,
        server: ServerPolicy,
    ) -> (Self, watch::Sender<ServerPolicy>) {
        let (tx, server) = watch::channel(server);
        let server = Cached::uncached(server);
        let p = Self { dst, server };
        (p, tx)
    }

    #[inline]
    pub(crate) fn protocol(&self) -> Protocol {
        self.server.borrow().protocol.clone()
    }

    #[inline]
    pub fn dst_addr(&self) -> OrigDstAddr {
        self.dst
    }

    #[inline]
    pub fn server_label(&self) -> ServerLabel {
        let s = self.server.borrow();
        ServerLabel {
            kind: s.kind.clone(),
            name: s.name.clone(),
        }
    }

    async fn changed(&mut self) {
        if self.server.changed().await.is_err() {
            // If the sender was dropped, then there can be no further changes.
            futures::future::pending::<()>().await;
        }
    }

    fn http_routes(&self) -> Option<Arc<[policy::HttpRoute]>> {
        let borrow = self.server.borrow();
        match &borrow.protocol {
            Protocol::Detect { http, .. } | Protocol::Http1(http) | Protocol::Http2(http) => {
                Some(http.routes.clone())
            }
            _ => None,
        }
    }

    /// Checks whether the server has any authorizations at all. If it does not,
    /// a denial error is returned.
    pub(crate) fn check_port_allowed(self) -> Result<Self, DeniedUnauthorized> {
        let server = self.server.borrow();

        if server.authorizations.is_empty() {
            return Err(DeniedUnauthorized {
                kind: server.kind.clone(),
                name: server.name.clone(),
            });
        }
        drop(server);

        Ok(self)
    }

    /// Checks whether the destination port's `AllowPolicy` is authorized to
    /// accept connections given the provided TLS state.
    pub(crate) fn check_authorized(
        &self,
        client_addr: Remote<ClientAddr>,
        tls: &tls::ConditionalServerTls,
    ) -> Result<Permit, DeniedUnauthorized> {
        let server = self.server.borrow();
        for authz in server.authorizations.iter() {
            if is_authorized(authz, client_addr, tls) {
                return Ok(Permit::new(self.dst, &*server, authz));
            }
        }

        Err(DeniedUnauthorized {
            kind: server.kind.clone(),
            name: server.name.clone(),
        })
    }
}

fn is_authorized(
    authz: &Authorization,
    client_addr: Remote<ClientAddr>,
    tls: &tls::ConditionalServerTls,
) -> bool {
    if !authz.networks.iter().any(|n| n.contains(&client_addr.ip())) {
        return false;
    }

    match authz.authentication {
        Authentication::Unauthenticated => true,

        Authentication::TlsUnauthenticated => {
            matches!(
                tls,
                tls::ConditionalServerTls::Some(tls::ServerTls::Established { .. })
            )
        }

        Authentication::TlsAuthenticated {
            ref identities,
            ref suffixes,
        } => match tls {
            tls::ConditionalServerTls::Some(tls::ServerTls::Established {
                client_id: Some(tls::server::ClientId(ref id)),
                ..
            }) => {
                identities.contains(id.as_str()) || suffixes.iter().any(|s| s.contains(id.as_str()))
            }
            _ => false,
        },
    }
}

// === impl Permit ===

impl Permit {
    fn new(dst: OrigDstAddr, server: &ServerPolicy, authz: &Authorization) -> Self {
        Self {
            dst,
            protocol: server.protocol.clone(),
            labels: AuthzLabels {
                kind: authz.kind.clone(),
                name: authz.name.clone(),
                server: ServerLabel {
                    kind: server.kind.clone(),
                    name: server.name.clone(),
                },
            },
        }
    }
}
