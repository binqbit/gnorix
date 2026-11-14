use anyhow::{Context, Result, bail, ensure};
use axum::http::uri::Authority;

const USAGE: &str = "usage: gnorix-near-demo [--port PORT] [--public-origin https://HOST[:PORT]]";

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    public_origin: Option<PublicOrigin>,
}

#[derive(Clone)]
struct PublicOrigin {
    host: String,
    origin: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8780,
            public_origin: None,
        }
    }
}

impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mut port = None;
        let mut public_origin = None;
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--port" if port.is_none() => {
                    let value = args.next().context(USAGE)?;
                    let parsed = value
                        .parse::<u16>()
                        .context("port must be within 1..=65535")?;
                    ensure!(parsed != 0, "port must be within 1..=65535");
                    port = Some(parsed);
                }
                "--public-origin" if public_origin.is_none() => {
                    public_origin = Some(PublicOrigin::parse(&args.next().context(USAGE)?)?);
                }
                _ => bail!(USAGE),
            }
        }
        Ok(Self {
            port: port.unwrap_or(8780),
            public_origin,
        })
    }

    pub fn public_origin(&self) -> Option<&str> {
        self.public_origin
            .as_ref()
            .map(|value| value.origin.as_str())
    }

    /// Validate the actual request headers, never proxy-supplied alternatives.
    /// An absent Origin permits navigation and local operational checks; writes
    /// additionally require the session token in the HTTP middleware.
    pub fn trusted_request(&self, host: Option<&str>, origin: Option<&str>) -> bool {
        let local = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ];
        let trusted_local = host.is_some_and(|host| local.iter().any(|value| value == host))
            && origin
                .is_none_or(|origin| local.iter().any(|host| origin == format!("http://{host}")));
        trusted_local
            || self.public_origin.as_ref().is_some_and(|public| {
                host == Some(public.host.as_str())
                    && origin.is_none_or(|origin| origin == public.origin)
            })
    }
}

impl PublicOrigin {
    fn parse(value: &str) -> Result<Self> {
        let host = value
            .strip_prefix("https://")
            .context("public origin must use HTTPS")?;
        ensure!(
            !host.contains(['/', '?', '#', '@']),
            "public origin must not contain credentials, a path, query, or fragment"
        );
        let authority: Authority = host.parse().context("invalid public origin authority")?;
        ensure!(
            !authority.host().is_empty(),
            "public origin must contain a host"
        );
        let suffix = &authority.as_str()[authority.host().len()..];
        let port = if suffix.is_empty() {
            None
        } else {
            let value = suffix
                .strip_prefix(':')
                .context("invalid public origin port")?;
            let port: u16 = value
                .parse()
                .context("public origin port must be within 1..=65535")?;
            ensure!(port != 0, "public origin port must be within 1..=65535");
            Some(port)
        };
        // Match the browser's serialized HTTPS origin: lowercase host and no
        // explicit default port. A non-default TLS port remains significant.
        let mut host = authority.host().to_ascii_lowercase();
        if let Some(port) = port.filter(|port| *port != 443) {
            host.push_str(&format!(":{port}"));
        }
        Ok(Self {
            origin: format!("https://{host}"),
            host,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(args: &[&str]) -> Result<Config> {
        Config::parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn arguments_accept_both_orders_and_reject_incomplete_or_duplicate_options() {
        assert_eq!(parse_config(&[]).unwrap().port, 8780);
        for args in [
            ["--port", "8800", "--public-origin", "https://gnorix.online"],
            ["--public-origin", "https://gnorix.online", "--port", "8800"],
        ] {
            let parsed = parse_config(&args).unwrap();
            assert_eq!(parsed.port, 8800);
            assert_eq!(parsed.public_origin(), Some("https://gnorix.online"));
        }
        for args in [
            vec!["--port"],
            vec!["--public-origin"],
            vec!["--port", "0"],
            vec!["--port", "65536"],
            vec!["--port", "8780", "--port", "8781"],
            vec![
                "--public-origin",
                "https://gnorix.online",
                "--public-origin",
                "https://gnorix.online",
            ],
            vec!["--unexpected"],
        ] {
            assert!(parse_config(&args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn public_origin_accepts_only_an_https_authority() {
        for value in [
            "http://gnorix.online",
            "https://",
            "https://:443",
            "https://user@gnorix.online",
            "https://user:pass@gnorix.online",
            "https://gnorix.online/",
            "https://gnorix.online/path",
            "https://gnorix.online?query",
            "https://gnorix.online#fragment",
            "https://gnorix.online:",
            "https://gnorix.online:abc",
            "https://gnorix.online:0",
            "https://gnorix.online:65536",
            "https://gnorix.online\r\nHost: evil.example",
        ] {
            assert!(PublicOrigin::parse(value).is_err(), "accepted {value}");
        }
        assert_eq!(
            PublicOrigin::parse("https://GNORIX.ONLINE:443")
                .unwrap()
                .origin,
            "https://gnorix.online"
        );
        assert_eq!(
            PublicOrigin::parse("https://gnorix.online:8443")
                .unwrap()
                .host,
            "gnorix.online:8443"
        );
    }

    #[test]
    fn local_origin_guard_rejects_cross_site_and_rebinding_requests() {
        let config = Config::default();
        assert!(config.trusted_request(Some("127.0.0.1:8780"), None));
        assert!(config.trusted_request(Some("localhost:8780"), Some("http://localhost:8780")));
        for (host, origin) in [
            (Some("example.com:8780"), None),
            (Some("localhost:8780"), Some("https://evil.example")),
            (Some("localhost:8780"), Some("null")),
            (Some("localhost:8781"), None),
            (Some("gnorix.online"), Some("https://gnorix.online")),
            (None, None),
        ] {
            assert!(!config.trusted_request(host, origin));
        }
    }

    #[test]
    fn public_origin_guard_requires_the_exact_host_and_origin_pair() {
        let config = parse_config(&["--public-origin", "https://gnorix.online"]).unwrap();
        assert!(config.trusted_request(Some("gnorix.online"), None));
        assert!(config.trusted_request(Some("gnorix.online"), Some("https://gnorix.online")));
        assert!(config.trusted_request(Some("127.0.0.1:8780"), Some("http://127.0.0.1:8780")));
        for (host, origin) in [
            (Some("evil.example"), Some("https://gnorix.online")),
            (Some("gnorix.online.evil.example"), None),
            (Some("www.gnorix.online"), Some("https://gnorix.online")),
            (Some("gnorix.online"), Some("http://gnorix.online")),
            (Some("gnorix.online"), Some("https://evil.example")),
            (Some("gnorix.online"), Some("https://gnorix.online:8443")),
            (Some("gnorix.online:8443"), Some("https://gnorix.online")),
            (Some("gnorix.online"), Some("null")),
            (Some("gnorix.online"), Some("http://localhost:8780")),
            (Some("localhost:8780"), Some("https://gnorix.online")),
            (None, Some("https://gnorix.online")),
        ] {
            assert!(
                !config.trusted_request(host, origin),
                "accepted {host:?} / {origin:?}"
            );
        }
        let config = parse_config(&["--public-origin", "https://gnorix.online:8443"]).unwrap();
        assert!(config.trusted_request(
            Some("gnorix.online:8443"),
            Some("https://gnorix.online:8443")
        ));
        assert!(!config.trusted_request(Some("gnorix.online"), Some("https://gnorix.online")));
    }
}
