//! Standalone binary for the `docs/adr/0009-account-system.md` reference
//! server. Not deployed anywhere — see `src/lib.rs`'s doc comment and the
//! ADR's "Status: Proposed" for why this is a reference implementation, not
//! production infrastructure. Configuration mirrors
//! `server/directory/src/main.rs`'s deliberately minimal shape.

use std::sync::Arc;

use ankai_accounts_server::app;
use ankai_accounts_server::db::Storage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path =
        std::env::var("ANKAI_ACCOUNTS_DB").unwrap_or_else(|_| "ankai-accounts.sqlite3".to_string());
    let bind_addr =
        std::env::var("ANKAI_ACCOUNTS_ADDR").unwrap_or_else(|_| "127.0.0.1:7421".to_string());

    let storage = Arc::new(Storage::open(&db_path)?);
    let app = app::router(storage);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    println!("ankai-accounts-server listening on {bind_addr} (db: {db_path})");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;

    Ok(())
}
