mod api;

use std::{env, path::PathBuf};

use anyhow::Context;
use kura_core::Core;
use kura_store::Store;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let data = PathBuf::from(env::var("KURA_DATA").unwrap_or_else(|_| "data".into()));
    std::fs::create_dir_all(&data)
        .with_context(|| format!("could not create the data folder {}", data.display()))?;
    let store = Store::open(&data.join("kura.db")).context("could not open the database")?;
    let core = Core::with_defaults(store)?;

    let listen = env::var("KURA_LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("could not listen on {listen}"))?;
    tracing::info!("Kura is running at http://{}", listener.local_addr()?);

    // Without its download workers the server is of no use, so either one ending ends both.
    tokio::select! {
        served = axum::serve(listener, api::router(core.clone())) => served?,
        worked = core.clone().run() => worked.context("the download workers stopped")?,
        synced = core.clone().run_auto_sync() => synced.context("Auto-Sync stopped")?,
    }
    Ok(())
}
