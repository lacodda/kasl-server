use std::net::SocketAddr;

use anyhow::{Context, Result};

/// Default bind address when `KASL_SERVER_ADDR` is not set.
const DEFAULT_ADDR: &str = "0.0.0.0:8080";

/// Email of the administrator created on a first run, when the operator named
/// none. Not a real address, and not meant to be: it is a name to sign in with,
/// and one that reads as "change me" rather than looking like someone's account.
const DEFAULT_ADMIN_EMAIL: &str = "admin@kasl.local";

/// Days one batch may carry. A month of backfill in a single request, which is
/// generous for the case it exists for and still bounded.
const DEFAULT_MAX_BATCH_DAYS: usize = 31;

/// Largest upload body accepted, in bytes. A day of dense activity is a few
/// kilobytes; a month of them, with room to spare, is well under this.
const DEFAULT_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Runtime configuration, read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the HTTP server binds to (`KASL_SERVER_ADDR`).
    pub addr: SocketAddr,
    /// PostgreSQL connection string (`DATABASE_URL`).
    pub database_url: String,
    /// Agents to provision on startup (`KASL_AGENTS`), as `email:token` pairs.
    /// The bootstrap way in until the admin UI issues tokens.
    pub agents: String,
    /// Days one batch upload may carry (`KASL_MAX_BATCH_DAYS`).
    pub max_batch_days: usize,
    /// Largest request body accepted (`KASL_MAX_BODY_BYTES`).
    pub max_body_bytes: usize,
    /// Bootstrap administrator (`KASL_ADMIN`), as `email:password`.
    pub admin: String,
    /// Email for the administrator generated on a first run (`KASL_ADMIN_EMAIL`).
    ///
    /// Only used when `KASL_ADMIN` is unset and the installation has no
    /// administrator at all: the server makes one with a random password and
    /// prints it once. The default is deliberately obvious rather than clever -
    /// an operator who never sets this should still recognize the account.
    pub admin_email: String,
    /// Whether session cookies carry `Secure` (`KASL_SECURE_COOKIES`).
    ///
    /// On by default, because a server holding a team's hours belongs behind
    /// TLS. Turned off only for a stand reached over plain http, where a
    /// `Secure` cookie is dropped by the browser and login silently does
    /// nothing at all.
    pub secure_cookies: bool,
    /// Whether to seed a fictional team on an empty database (`KASL_DEMO`).
    ///
    /// Off by default. On, the server fills an empty database with the demo
    /// team and refuses to start on one that holds anybody real (ADR 0013).
    pub demo: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// A configuration with every limit at its default. Used where the limits
    /// are not the subject - the router built for tests, for instance - so the
    /// defaults live in one place rather than being restated.
    pub fn defaults_for_database(database_url: String) -> Self {
        Self {
            addr: DEFAULT_ADDR.parse().expect("the default address is valid"),
            database_url,
            agents: String::new(),
            max_batch_days: DEFAULT_MAX_BATCH_DAYS,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            admin: String::new(),
            admin_email: DEFAULT_ADMIN_EMAIL.to_string(),
            secure_cookies: true,
            demo: false,
        }
    }

    /// The environment is passed in as a lookup so tests can supply their own.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let addr = lookup("KASL_SERVER_ADDR").unwrap_or_else(|| DEFAULT_ADDR.to_string());
        let addr = addr
            .parse()
            .with_context(|| format!("KASL_SERVER_ADDR is not a valid socket address: {addr}"))?;
        let database_url = lookup("DATABASE_URL").context("DATABASE_URL is not set (e.g. postgres://kasl:kasl@localhost:5432/kasl)")?;
        let agents = lookup("KASL_AGENTS").unwrap_or_default();
        let max_batch_days = positive("KASL_MAX_BATCH_DAYS", &lookup, DEFAULT_MAX_BATCH_DAYS)?;
        let max_body_bytes = positive("KASL_MAX_BODY_BYTES", &lookup, DEFAULT_MAX_BODY_BYTES)?;
        let admin = lookup("KASL_ADMIN").unwrap_or_default();
        let admin_email = lookup("KASL_ADMIN_EMAIL").unwrap_or_else(|| DEFAULT_ADMIN_EMAIL.to_string());
        let secure_cookies = boolean("KASL_SECURE_COOKIES", &lookup, true)?;
        let demo = boolean("KASL_DEMO", &lookup, false)?;
        Ok(Self {
            addr,
            database_url,
            agents,
            max_batch_days,
            max_body_bytes,
            admin,
            admin_email,
            secure_cookies,
            demo,
        })
    }
}

/// Reads a flag written the way an operator would write one.
///
/// Refuses anything else rather than guessing: reading `KASL_SECURE_COOKIES=no`
/// as true would turn a typo into a server that quietly cannot be logged into.
fn boolean(key: &str, lookup: &impl Fn(&str) -> Option<String>, default: bool) -> Result<bool> {
    let Some(raw) = lookup(key) else { return Ok(default) };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => anyhow::bail!("{key} is not a yes/no value: {other}"),
    }
}

/// Reads a limit, refusing zero: a limit of nothing accepts nothing, and a
/// server that silently rejects every upload is worse than one that will not
/// start.
fn positive(key: &str, lookup: &impl Fn(&str) -> Option<String>, default: usize) -> Result<usize> {
    let Some(raw) = lookup(key) else { return Ok(default) };
    let value: usize = raw.parse().with_context(|| format!("{key} is not a positive whole number: {raw}"))?;
    if value == 0 {
        anyhow::bail!("{key} must be greater than zero");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string())
    }

    #[test]
    fn defaults_the_bind_address() {
        let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl")])).expect("config should build with only DATABASE_URL set");
        assert_eq!(config.addr, DEFAULT_ADDR.parse().unwrap());
        assert_eq!(config.database_url, "postgres://localhost/kasl");
    }

    #[test]
    fn reads_the_bind_address_override() {
        let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_SERVER_ADDR", "127.0.0.1:9090")]))
            .expect("config should accept a valid override");
        assert_eq!(config.addr, "127.0.0.1:9090".parse().unwrap());
    }

    #[test]
    fn requires_database_url() {
        let error = Config::from_lookup(env(&[])).unwrap_err();
        assert!(error.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn limits_have_defaults_and_can_be_overridden() {
        let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl")])).unwrap();
        assert_eq!(config.max_batch_days, DEFAULT_MAX_BATCH_DAYS);
        assert_eq!(config.max_body_bytes, DEFAULT_MAX_BODY_BYTES);

        let config = Config::from_lookup(env(&[
            ("DATABASE_URL", "postgres://localhost/kasl"),
            ("KASL_MAX_BATCH_DAYS", "7"),
            ("KASL_MAX_BODY_BYTES", "1048576"),
        ]))
        .unwrap();
        assert_eq!(config.max_batch_days, 7);
        assert_eq!(config.max_body_bytes, 1048576);
    }

    #[test]
    fn a_limit_of_zero_is_refused() {
        // Zero accepts nothing. A server that silently rejects every upload is
        // worse than one that refuses to start and says why.
        let error = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_MAX_BATCH_DAYS", "0")])).unwrap_err();
        assert!(error.to_string().contains("KASL_MAX_BATCH_DAYS"), "{error}");

        let error = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_MAX_BODY_BYTES", "nope")])).unwrap_err();
        assert!(error.to_string().contains("KASL_MAX_BODY_BYTES"), "{error}");
    }

    #[test]
    fn secure_cookies_default_on_and_refuse_a_typo() {
        let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl")])).unwrap();
        assert!(config.secure_cookies, "TLS is the assumption; opting out has to be deliberate");

        for value in ["0", "false", "no", "off", "OFF"] {
            let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_SECURE_COOKIES", value)])).unwrap();
            assert!(!config.secure_cookies, "`{value}` should turn it off");
        }

        // The failure mode this guards: a value nobody parses as false, read as
        // true, giving a server that cannot be logged into over plain http with
        // nothing in the log to say why.
        let error = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_SECURE_COOKIES", "nope")])).unwrap_err();
        assert!(error.to_string().contains("KASL_SECURE_COOKIES"), "{error}");
    }

    #[test]
    fn the_demo_is_off_unless_asked_for() {
        // The failure this guards is the quiet one: a server that seeds a
        // fictional team because a flag defaulted the wrong way.
        let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl")])).unwrap();
        assert!(!config.demo);

        let config = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_DEMO", "true")])).unwrap();
        assert!(config.demo);

        let error = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_DEMO", "maybe")])).unwrap_err();
        assert!(error.to_string().contains("KASL_DEMO"), "{error}");
    }

    #[test]
    fn rejects_a_malformed_bind_address() {
        let error = Config::from_lookup(env(&[("DATABASE_URL", "postgres://localhost/kasl"), ("KASL_SERVER_ADDR", "not-an-address")])).unwrap_err();
        assert!(error.to_string().contains("KASL_SERVER_ADDR"));
    }
}
