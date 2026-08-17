use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kasl_server::{app, config, import, provision};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

/// Team server for kasl.
///
/// Runs the server when given no subcommand, which is what a container's
/// entrypoint does and what every existing deployment expects.
#[derive(Debug, Parser)]
#[command(name = "kasl-server", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Import an employee's local kasl history from their SQLite database.
    Import(ImportArgs),
}

#[derive(Debug, clap::Args)]
struct ImportArgs {
    /// Path to the agent's database file (kasl's own `kasl.db`).
    #[arg(long, value_name = "PATH")]
    db: std::path::PathBuf,
    /// Email of the user to import into. The account must already exist.
    #[arg(long, value_name = "EMAIL")]
    user: String,
    /// UTC offset the history was recorded in, e.g. `-03:00`.
    ///
    /// Required, with no default: kasl stores bare wall-clock time, so nothing
    /// in the file says which offset it was. A wrong guess here silently shifts
    /// a year of someone's hours, so the answer comes from the person who knows
    /// (ADR 0006).
    ///
    /// `allow_hyphen_values` because the common case starts with one: without
    /// it `--timezone -03:00` is read as an unknown flag `-0`, and the operator
    /// is refused for typing exactly what the documentation shows.
    #[arg(long, value_name = "OFFSET", value_parser = parse_offset, allow_hyphen_values = true)]
    timezone: chrono::FixedOffset,
    /// Import only days on or after this date (`YYYY-MM-DD`).
    ///
    /// With `--until`, this is how an employee who moved between time zones is
    /// imported correctly: one run per stretch, each with its own offset.
    #[arg(long, value_name = "DATE")]
    since: Option<chrono::NaiveDate>,
    /// Import only days on or before this date (`YYYY-MM-DD`).
    #[arg(long, value_name = "DATE")]
    until: Option<chrono::NaiveDate>,
    /// Read and report what would be imported, without writing anything.
    #[arg(long)]
    dry_run: bool,
}

/// Parses `-03:00`, `+05:30`, or `Z`.
fn parse_offset(raw: &str) -> Result<chrono::FixedOffset, String> {
    // Parsed by borrowing a full timestamp's machinery: the offset alone has no
    // parser in chrono, and hand-rolling one invites the classic sign mistake.
    chrono::DateTime::parse_from_rfc3339(&format!("2000-01-01T00:00:00{}", if raw == "Z" { "Z".to_string() } else { raw.to_string() }))
        .map(|time| *time.offset())
        .map_err(|_| format!("not a UTC offset: {raw} (expected something like -03:00, +05:30 or Z)"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("kasl_server=info,tower_http=info")))
        .init();

    let config = config::Config::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;
    let migrator = kasl_server::migrator();
    // Worth a line: on a fresh install this is where the schema appears, and
    // on an upgrade it is the first thing to check when something looks off.
    let target = migrator.migrations.last().map(|m| m.version).unwrap_or_default();
    migrator.run(&pool).await.context("failed to apply database migrations")?;
    tracing::info!(version = target, "database schema is up to date");

    match cli.command {
        Some(Command::Import(args)) => run_import(&pool, args).await,
        None => serve(pool, config).await,
    }
}

async fn run_import(pool: &sqlx::PgPool, args: ImportArgs) -> Result<()> {
    // Resolved before the file is read: a typo in the email should cost the
    // operator a second, not the time it takes to parse a year of history.
    let user_id = import::resolve_user(pool, &args.user).await?;

    let (days, summary) = import::read_agent_db(&args.db)?;
    println!(
        "read {} workdays, {} pauses, {} tasks from {}",
        summary.days,
        summary.pauses,
        summary.tasks,
        args.db.display()
    );

    let days = import::within(days, args.since, args.until);
    if args.since.is_some() || args.until.is_some() {
        println!("selected {} days in range", days.len());
    }
    if summary.skipped_deleted_tasks > 0 {
        println!("skipped {} tasks the employee had deleted", summary.skipped_deleted_tasks);
    }
    if summary.skipped_unreadable > 0 {
        println!("skipped {} rows whose timestamps could not be read", summary.skipped_unreadable);
    }

    if args.dry_run {
        println!("dry run: nothing was written");
        return Ok(());
    }

    let written = import::write_days(pool, user_id, &days, args.timezone).await?;
    // The offset is echoed because it is the one thing that cannot be checked
    // afterwards by looking at the data: every instant is now stated relative
    // to it, and a wrong one looks entirely plausible.
    println!("imported {written} days as {} at {}", args.user, args.timezone);

    Ok(())
}

async fn serve(pool: sqlx::PgPool, config: config::Config) -> Result<()> {
    let seeds = provision::parse_seeds(&config.agents)?;
    provision::apply_seeds(&pool, &seeds).await?;

    let listener = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("failed to bind {}", config.addr))?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), addr = %config.addr, max_batch_days = config.max_batch_days, max_body_bytes = config.max_body_bytes, "kasl-server listening");

    axum::serve(listener, app::router_with(pool, &config))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install the shutdown signal handler");
        return;
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_utc_offsets_in_the_forms_an_operator_would_type() {
        assert_eq!(parse_offset("-03:00").unwrap().local_minus_utc(), -3 * 3600);
        assert_eq!(parse_offset("+05:30").unwrap().local_minus_utc(), 5 * 3600 + 1800);
        assert_eq!(parse_offset("Z").unwrap().local_minus_utc(), 0);
    }

    #[test]
    fn refuses_something_that_is_not_an_offset() {
        // Notably a zone name: accepting it would imply DST handling this does
        // not do, and quietly importing a year at the wrong hour.
        assert!(parse_offset("America/Asuncion").is_err());
        assert!(parse_offset("-3").is_err());
        assert!(parse_offset("").is_err());
    }

    #[test]
    fn the_cli_still_runs_the_server_without_a_subcommand() {
        // The container entrypoint passes no arguments; that must keep meaning
        // "serve" now that subcommands exist.
        let cli = Cli::try_parse_from(["kasl-server"]).expect("no arguments must remain valid");
        assert!(cli.command.is_none());
    }

    #[test]
    fn the_import_requires_a_timezone() {
        let missing = Cli::try_parse_from(["kasl-server", "import", "--db", "kasl.db", "--user", "a@b.c"]);
        assert!(missing.is_err(), "an import without an offset must not start");

        let complete = Cli::try_parse_from(["kasl-server", "import", "--db", "kasl.db", "--user", "a@b.c", "--timezone", "-03:00"]);
        assert!(complete.is_ok(), "and with one it must");
    }
}
