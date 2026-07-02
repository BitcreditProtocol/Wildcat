// ----- standard library imports
// ----- extra library imports
use bcr_wdc_treasury_service::{
    ebill::Repository,
    persistence::{sqlx, surreal},
};

// ----- local imports

// ----- end imports:

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: bcr_wdc_treasury_service::config::App,
}

#[tokio::main]
async fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("TREASURY_SERVICE"))
        .build()
        .expect("Failed to build config");
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .expect("Failed to parse migrate config");
    // Connect to SurrealDB (source)
    let surreal_ebill = surreal::DBEbill::new(cfg.appcfg.ebill.db)
        .await
        .expect("Failed to connect to SurrealDB");
    // Read all ebill mint_ops
    let ops = surreal_ebill
        .mint_list_all()
        .await
        .expect("Failed to list ebill mint_ops from SurrealDB");
    println!("Found {} ebill mint_ops in SurrealDB", ops.len());
    if !ops.is_empty() {
        let sample: Vec<_> = ops.iter().map(|op| op.uid).take(3).collect();
        println!("Sample UIDs: {:?}", sample);
    }
    if dry_run {
        println!(
            "DRY RUN: Would migrate {} ebill mint_ops to PostgreSQL",
            ops.len()
        );
        return;
    }
    // Connect to PostgreSQL (destination) — DBEbill::new creates pool and runs migrations
    let sqlx_ebill = sqlx::DBEbill::new(cfg.appcfg.ebill.new)
        .await
        .expect("Failed to connect to PostgreSQL");
    for op in ops {
        let uid = op.uid;
        if let Err(e) = sqlx_ebill.mint_store(op).await {
            println!("Skipping mint_op {uid}: failed with {e}");
        }
    }
    println!("Migration complete");
}
