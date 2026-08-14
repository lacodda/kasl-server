//! Getting the first agents onto a server that has no admin UI yet.
//!
//! Until accounts and token issuing arrive with the people milestone, the
//! operator declares agents in the environment:
//!
//! ```text
//! KASL_AGENTS=alice@example.com:s3cr3t-token,bob@example.com:another-token
//! ```
//!
//! On startup each entry becomes a user and an agent holding that token's
//! hash. Deliberately not a public enrollment endpoint: a server whose only
//! way in is a secret the operator already knows cannot be joined by anyone
//! who does not.
//!
//! This is a bootstrap, and it says so - when the admin UI issues tokens, the
//! variable stops being the way in and the ingest contract does not change.

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::auth::hash_token;

/// One `email:token` pair from the environment.
#[derive(Debug, PartialEq, Eq)]
pub struct AgentSeed {
    pub email: String,
    pub token: String,
}

/// Parses `KASL_AGENTS`. An empty or absent value yields nothing, which is the
/// normal state of a server whose agents are already in the database.
pub fn parse_seeds(raw: &str) -> Result<Vec<AgentSeed>> {
    let mut seeds = Vec::new();

    for entry in raw.split(',').map(str::trim).filter(|entry| !entry.is_empty()) {
        // `rsplit_once` so a colon inside the token - plausible in a generated
        // secret - stays part of it.
        let (email, token) = entry
            .split_once(':')
            .with_context(|| format!("KASL_AGENTS entry `{entry}` is not `email:token`"))?;
        let (email, token) = (email.trim(), token.trim());

        if email.is_empty() || token.is_empty() {
            anyhow::bail!("KASL_AGENTS entry `{entry}` has an empty email or token");
        }

        seeds.push(AgentSeed {
            email: email.to_string(),
            token: token.to_string(),
        });
    }

    Ok(seeds)
}

/// Creates or updates the declared users and agents.
///
/// Idempotent: restarting the server with the same variable changes nothing,
/// and rotating a token in the variable rotates it in the database.
pub async fn apply_seeds(pool: &PgPool, seeds: &[AgentSeed]) -> Result<()> {
    for seed in seeds {
        let mut tx = pool.begin().await?;

        // The display name is the local part until someone sets a real one;
        // the admin UI will own this field later.
        let display_name = seed.email.split('@').next().unwrap_or(&seed.email);

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (email, display_name) VALUES ($1, $2)
             ON CONFLICT (lower(email)) DO UPDATE SET active = true
             RETURNING id",
        )
        .bind(&seed.email)
        .bind(display_name)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("failed to provision the user for {}", seed.email))?;

        // One seeded agent per user, identified by its name: re-running with a
        // new token replaces the hash instead of leaving the old one valid.
        sqlx::query(
            "INSERT INTO agents (user_id, name, token_hash, revoked_at) VALUES ($1, 'seeded', $2, NULL)
             ON CONFLICT (token_hash) DO NOTHING",
        )
        .bind(user_id)
        .bind(hash_token(&seed.token))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to provision the agent for {}", seed.email))?;

        sqlx::query("UPDATE agents SET revoked_at = now() WHERE user_id = $1 AND name = 'seeded' AND token_hash <> $2 AND revoked_at IS NULL")
            .bind(user_id)
            .bind(hash_token(&seed.token))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to revoke the previous token for {}", seed.email))?;

        tx.commit().await?;
    }

    if !seeds.is_empty() {
        // Count only: the tokens are secrets and the addresses are personal.
        tracing::info!(agents = seeds.len(), "provisioned agents from KASL_AGENTS");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_several_entries() {
        let seeds = parse_seeds("alice@example.test:token-a, bob@example.test:token-b").unwrap();
        assert_eq!(
            seeds,
            vec![
                AgentSeed {
                    email: "alice@example.test".into(),
                    token: "token-a".into()
                },
                AgentSeed {
                    email: "bob@example.test".into(),
                    token: "token-b".into()
                },
            ]
        );
    }

    #[test]
    fn an_absent_variable_provisions_nothing() {
        assert!(parse_seeds("").unwrap().is_empty());
        assert!(parse_seeds("  ,  ").unwrap().is_empty());
    }

    #[test]
    fn a_colon_inside_the_token_survives() {
        let seeds = parse_seeds("alice@example.test:ab:cd").unwrap();
        assert_eq!(seeds[0].token, "ab:cd", "only the first colon separates");
    }

    #[test]
    fn malformed_entries_name_themselves() {
        let error = parse_seeds("alice@example.test").unwrap_err().to_string();
        assert!(error.contains("alice@example.test"), "the message should quote the entry: {error}");

        assert!(parse_seeds("alice@example.test:").is_err(), "an empty token is not usable");
        assert!(parse_seeds(":token").is_err(), "an agent with no user has nobody to report for");
    }
}
