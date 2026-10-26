//! `mise food ...`: inspect and curate the food model.

use anyhow::Result;
use clap::Subcommand;
use mise_ingest::curate::{self, Op};
use sqlx::PgPool;

#[derive(Subcommand)]
pub enum FoodCommand {
    /// Show a food: parent, children, aliases, related foods, facts and usage.
    Show { name: String },
    /// Add a food, brand, variety, cut or form.
    Add {
        name: String,
        /// Make it a kind of this food.
        #[arg(long)]
        parent: Option<String>,
        /// food, brand, variety, cut, form or proprietary.
        #[arg(long, default_value = "food")]
        kind: String,
        /// Other names for it (repeatable).
        #[arg(long = "alias")]
        aliases: Vec<String>,
    },
    /// Make FOOD a kind of PARENT.
    SetParent { food: String, parent: String },
    /// Make FOOD top-level.
    Unparent { food: String },
    /// Add another name for FOOD.
    Alias { food: String, alias: String },
    /// Remove the related link between FOOD and OTHER.
    Unrelate { food: String, other: String },
    /// FROM and INTO are the same food: move everything to INTO and delete FROM.
    Merge { from: String, into: String },
    /// Set a property, e.g. `default_flexibility strict` or `store_section Dairy`.
    Set {
        food: String,
        key: String,
        /// A JSON value or plain text.
        value: String,
    },
    /// Apply a JSON file of operations (see docs/curation.md), all or nothing.
    Apply { file: std::path::PathBuf },
}

pub async fn run(pool: &PgPool, cmd: FoodCommand) -> Result<()> {
    let op = match cmd {
        FoodCommand::Show { name } => return show(pool, &name).await,
        FoodCommand::Apply { file } => {
            for line in curate::apply_file(pool, &file).await? {
                println!("{line}");
            }
            return Ok(());
        }
        FoodCommand::Add { name, parent, kind, aliases } => Op::Add { name, parent, kind, aliases },
        FoodCommand::SetParent { food, parent } => Op::SetParent { food, parent },
        FoodCommand::Unparent { food } => Op::Unparent { food },
        FoodCommand::Alias { food, alias } => Op::Alias { food, alias },
        FoodCommand::Unrelate { food, other } => Op::Unrelate { food, other },
        FoodCommand::Merge { from, into } => Op::Merge { from, into },
        FoodCommand::Set { food, key, value } => {
            let value = serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));
            Op::Set { food, key, value }
        }
    };
    println!("{}", curate::apply(pool, &op).await?);
    Ok(())
}

async fn show(pool: &PgPool, name: &str) -> Result<()> {
    let row: Option<(uuid::Uuid, String, String, String, Option<String>, serde_json::Value)> = sqlx::query_as(
        "SELECT f.id, f.name, f.kind::text, f.origin, p.name, f.props FROM foods f
         LEFT JOIN foods p ON p.id = f.parent_id
         LEFT JOIN food_aliases a ON a.food_id = f.id
         WHERE a.alias = $1 OR f.slug = $2 OR lower(f.name) = lower($3) LIMIT 1",
    )
    .bind(mise_core::food_names::singular_key(name))
    .bind(mise_core::text::slugify(name))
    .bind(name)
    .fetch_optional(pool)
    .await?;
    let Some((id, name, kind, origin, parent, props)) = row else {
        anyhow::bail!("no food named '{name}'");
    };
    let list = |sql: &'static str| async move {
        sqlx::query_scalar::<_, String>(sql).bind(id).fetch_all(pool).await.map(|v| v.join(", "))
    };
    println!("{name}  ({kind}, origin {origin})");
    println!("  parent:   {}", parent.unwrap_or_else(|| "—".into()));
    println!("  children: {}", list("SELECT name FROM foods WHERE parent_id = $1 ORDER BY name").await?);
    println!("  aliases:  {}", list("SELECT alias FROM food_aliases WHERE food_id = $1 ORDER BY alias").await?);
    println!(
        "  related:  {}",
        list("SELECT f.name FROM food_relations r JOIN foods f ON f.id = CASE WHEN r.from_food = $1 THEN r.to_food ELSE r.from_food END
              WHERE (r.from_food = $1 OR r.to_food = $1) AND r.kind = 'related'")
        .await?
    );
    if props != serde_json::json!({}) {
        println!("  props:    {props}");
    }
    let uses: i64 = sqlx::query_scalar("SELECT count(*) FROM ingredient_lines WHERE food_id = $1").bind(id).fetch_one(pool).await?;
    println!("  used in:  {uses} ingredient lines");
    let facts: Vec<(String, String, serde_json::Value)> =
        sqlx::query_as("SELECT kind, tier::text, value FROM food_facts WHERE food_id = $1 ORDER BY kind, tier")
            .bind(id)
            .fetch_all(pool)
            .await?;
    for (kind, tier, value) in facts {
        let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
        println!("  {kind} [{tier}]: {}", text.chars().take(160).collect::<String>());
    }
    Ok(())
}
