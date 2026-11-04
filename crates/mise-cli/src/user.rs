//! `mise user ...`: manage accounts from the server, e.g. to get back in after losing the admin password.
//! Passwords are read from standard input (one line), never from the command line, so they stay out of shell
//! history: `echo 'new password' | mise user set-password paul`.

use std::io::BufRead;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use mise_api::auth::{self, routes::NewUser};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum UserCommand {
    /// List accounts.
    List,
    /// Add an account; the password is read from standard input (leave the line empty for single sign-on only).
    Add {
        username: String,
        #[arg(long)]
        admin: bool,
        #[arg(long)]
        email: Option<String>,
        /// Join the household with this name (created if missing); default: a new household of its own.
        #[arg(long)]
        household: Option<String>,
    },
    /// Set an account's password from standard input and sign it out everywhere.
    SetPassword { username: String },
    /// Make an account an admin, or (with --off) not.
    Admin {
        username: String,
        #[arg(long)]
        off: bool,
    },
}

fn read_password() -> Result<String> {
    eprintln!("Password (one line on standard input):");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the password")?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

async fn user_id(pool: &PgPool, username: &str) -> Result<Uuid> {
    sqlx::query_scalar("SELECT id FROM users WHERE lower(username) = lower($1)")
        .bind(username)
        .fetch_optional(pool)
        .await?
        .with_context(|| format!("no account named {username:?}"))
}

pub async fn run(pool: &PgPool, cmd: UserCommand) -> Result<()> {
    match cmd {
        UserCommand::List => {
            let rows: Vec<(String, String, bool, bool, String, bool)> = sqlx::query_as(
                "SELECT u.username, u.display_name, u.is_admin, u.disabled, h.name, u.password_hash IS NOT NULL
                 FROM users u JOIN households h ON h.id = u.household_id ORDER BY lower(u.username)",
            )
            .fetch_all(pool)
            .await?;
            if rows.is_empty() {
                println!(
                    "No accounts yet: the first visit to the web app creates the admin account."
                );
            }
            for (username, name, admin, disabled, household, password) in rows {
                let flags = [
                    admin.then_some("admin"),
                    disabled.then_some("off"),
                    (!password).then_some("sso only"),
                ];
                let flags: Vec<&str> = flags.into_iter().flatten().collect();
                println!(
                    "{username:<20} {name:<24} {household:<24} {}",
                    flags.join(", ")
                );
            }
        }
        UserCommand::Add {
            username,
            admin,
            email,
            household,
        } => {
            let password = read_password()?;
            let household_id = match &household {
                Some(name) => {
                    sqlx::query_scalar::<_, Uuid>(
                        "SELECT id FROM households WHERE name = $1 ORDER BY created_at LIMIT 1",
                    )
                    .bind(name)
                    .fetch_optional(pool)
                    .await?
                }
                None => None,
            };
            let r = NewUser {
                username: username.clone(),
                password: (!password.is_empty()).then_some(password),
                display_name: None,
                email,
                is_admin: admin,
                household_id,
                household_name: household,
            };
            auth::routes::add_user(pool, r)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("Added {username}.");
        }
        UserCommand::SetPassword { username } => {
            let id = user_id(pool, &username).await?;
            let password = read_password()?;
            auth::validate_password(&password).map_err(|e| anyhow::anyhow!("{e}"))?;
            let hash = auth::hash_password(&password)?;
            sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
                .bind(id)
                .bind(hash)
                .execute(pool)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                .bind(id)
                .execute(pool)
                .await?;
            println!("Password set for {username}; it was signed out everywhere.");
        }
        UserCommand::Admin { username, off } => {
            let id = user_id(pool, &username).await?;
            if off {
                let others: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM users WHERE is_admin AND NOT disabled AND id <> $1",
                )
                .bind(id)
                .fetch_one(pool)
                .await?;
                if others == 0 {
                    bail!("{username} is the last admin");
                }
            }
            sqlx::query("UPDATE users SET is_admin = $2 WHERE id = $1")
                .bind(id)
                .bind(!off)
                .execute(pool)
                .await?;
            println!(
                "{username} is {}an admin.",
                if off { "no longer " } else { "now " }
            );
        }
    }
    Ok(())
}
