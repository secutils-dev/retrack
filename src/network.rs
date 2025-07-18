mod dns_resolver;
mod ip_addr_ext;
mod smtp;

pub use self::{
    dns_resolver::{DnsResolver, TokioDnsResolver},
    ip_addr_ext::IpAddrExt,
    smtp::{Smtp, SmtpTransport},
};
use crate::config::Config;
use anyhow::{Context, bail};
use lettre::{
    message::Mailbox,
    transport::smtp::{authentication::Credentials, client::Tls},
};
use std::{net::IpAddr, str::FromStr, time::Duration};
use tracing::error;
use url::{Host, Url};

/// Network utilities.
pub struct Network<DR: DnsResolver> {
    pub resolver: DR,
    pub smtp: Option<Smtp>,
}

impl Network<TokioDnsResolver> {
    /// Creates a new `Network` instance with Tokio DNS resolver and Tokio SMTP transport.
    pub fn create(config: &Config) -> anyhow::Result<Self> {
        let smtp = if let Some(smtp_config) = config.smtp.clone() {
            if smtp_config.throttle_delay.lt(&Duration::from_secs(1)) {
                bail!("SMTP throttle delay cannot be less than 1s.");
            }

            if let Some(ref catch_all_config) = smtp_config.catch_all {
                Mailbox::from_str(catch_all_config.recipient.as_str())
                    .context("Cannot parse SMTP catch-all recipient.")?;
            }

            let mut smtp_builder = SmtpTransport::relay(&smtp_config.host)?.credentials(
                Credentials::new(smtp_config.username.clone(), smtp_config.password.clone()),
            );

            if let Some(port) = smtp_config.port {
                smtp_builder = smtp_builder.port(port);
            }

            if smtp_config.no_tls {
                smtp_builder = smtp_builder.tls(Tls::None);
            }
            Some(Smtp::new(smtp_builder.build(), smtp_config))
        } else {
            None
        };

        Ok(Network {
            resolver: TokioDnsResolver::create(),
            smtp,
        })
    }
}

impl<DR: DnsResolver> Network<DR> {
    /// Checks if the provided URL is a publicly accessible web URL.
    pub async fn is_public_web_url(&self, url: &Url) -> bool {
        if url.scheme() != "http" && url.scheme() != "https" {
            return false;
        }

        // Checks if the specific hostname is a domain and public (not pointing to the local network).
        match url.host() {
            Some(Host::Domain(domain)) => match self.resolver.lookup_ip(domain).await {
                Ok(lookup) => lookup.iter().all(|ip| IpAddrExt::is_global(&ip)),
                Err(err) => {
                    error!("Cannot resolve domain ({domain}) to IP: {err}");
                    false
                }
            },
            Some(Host::Ipv4(ip)) => IpAddrExt::is_global(&IpAddr::V4(ip)),
            Some(Host::Ipv6(ip)) => IpAddrExt::is_global(&IpAddr::V6(ip)),
            None => false,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::Network;
    use std::net::Ipv4Addr;
    use trust_dns_resolver::{
        Name,
        error::{ResolveError, ResolveErrorKind},
        proto::rr::{RData, Record, rdata::A},
    };
    use url::Url;

    pub use super::{dns_resolver::tests::*, smtp::tests::*};

    #[tokio::test]
    async fn correctly_checks_public_web_urls() -> anyhow::Result<()> {
        let public_network = Network {
            resolver: MockResolver::new_with_records::<1>(vec![Record::from_rdata(
                Name::new(),
                300,
                RData::A(A(Ipv4Addr::new(172, 32, 0, 2))),
            )]),
            smtp: None,
        };

        // Only `http` and `https` should be supported.
        for (protocol, is_supported) in [
            ("ftp", false),
            ("wss", false),
            ("http", true),
            ("https", true),
        ] {
            let url = Url::parse(&format!("{}://retrack.dev/my-page", protocol))?;
            assert_eq!(public_network.is_public_web_url(&url).await, is_supported);
        }

        // Hosts that resolve to local IPs aren't supported.
        let url = Url::parse("https://retrack.dev/my-page")?;
        let local_network = Network {
            resolver: MockResolver::new_with_records::<1>(vec![Record::from_rdata(
                Name::new(),
                300,
                RData::A(A(Ipv4Addr::new(127, 0, 0, 1))),
            )]),
            smtp: None,
        };
        for (network, is_supported) in [(public_network, true), (local_network, false)] {
            assert_eq!(network.is_public_web_url(&url).await, is_supported);
        }

        // Hosts that fail to resolve aren't supported and gracefully handled.
        let broken_network = Network {
            resolver: MockResolver::new_with_error(ResolveError::from(ResolveErrorKind::Message(
                "can not lookup IPs",
            ))),
            smtp: None,
        };
        assert!(!broken_network.is_public_web_url(&url).await);

        Ok(())
    }

    #[tokio::test]
    async fn correctly_checks_public_ips() -> anyhow::Result<()> {
        let network = Network {
            resolver: MockResolver::new(),
            smtp: None,
        };
        for (ip, is_supported) in [
            ("127.0.0.1", false),
            ("10.254.0.0", false),
            ("192.168.10.65", false),
            ("172.16.10.65", false),
            ("[2001:0db8:85a3:0000:0000:8a2e:0370:7334]", false),
            ("[::1]", false),
            ("217.88.39.143", true),
            ("[2001:1234:abcd:5678:0221:2fff:feb5:6e10]", true),
        ] {
            let url = Url::parse(&format!("http://{}/my-page", ip))?;
            assert_eq!(network.is_public_web_url(&url).await, is_supported);
        }

        Ok(())
    }
}
