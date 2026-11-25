mod api;
mod proxy;

use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode, time::Duration};

use anyhow::Context;
use kura_core::Core;
use kura_store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const USAGE: &str = "\
Kura: a self-hosted downloader for AniWorld and SerienStream.

Usage:
  kura               Start the server
  kura healthcheck   Exit 0 when the server on KURA_LISTEN answers, 1 otherwise
  kura --version     Print the version
  kura --help        Print this help

Environment:
  KURA_LISTEN            Address and port (default 127.0.0.1:8787)
  KURA_DATA              Folder for the database (default data)
  KURA_LIBRARY_DIR       Folder the library setup suggests
  KURA_NAMING_TEMPLATE   File name template while the settings set none
  KURA_TRUSTED_PROXIES   Reverse proxies whose X-Forwarded-For is believed, e.g.
                         172.18.0.0/16 or 10.0.0.5,::1 (default: none)
";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => match serve().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("kura: {err:#}");
                ExitCode::FAILURE
            }
        },
        ["healthcheck"] => healthcheck().await,
        ["--version" | "-V" | "version"] => {
            println!("kura {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ["--help" | "-h" | "help"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("kura: unknown arguments {args:?}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn listen_address() -> String {
    env::var("KURA_LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".into())
}

async fn serve() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let data = PathBuf::from(env::var("KURA_DATA").unwrap_or_else(|_| "data".into()));
    std::fs::create_dir_all(&data)
        .with_context(|| format!("could not create the data folder {}", data.display()))?;
    keep_private(&data);
    let store = Store::open(&data.join("kura.db")).context("could not open the database")?;
    // The database exists now, also on the very first start; its folder was closed already.
    keep_private(&data);
    let proxies = proxy::TrustedProxies::from_env()
        .map_err(|err| anyhow::anyhow!("KURA_TRUSTED_PROXIES: {err}"))?;
    let core = Core::with_defaults(store)?;
    core.set_data_dir(data.clone());
    // Asks GitHub for a newer release now, so the top bar can show it from the first page.
    core.newer_release();

    let listen = listen_address();
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("could not listen on {listen}"))?;
    tracing::info!(
        "Kura {} is running at http://{}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr()?
    );
    if !proxies.is_empty() {
        tracing::info!("believing X-Forwarded-For from {proxies}");
    }

    // Without its download workers the server is of no use, so either one ending ends both.
    // Stopping drops the running downloads, which ends ffmpeg and removes partial files;
    // they start over on the next start.
    tokio::select! {
        served = api::serve(listener, core.clone(), proxies) => served?,
        worked = core.clone().run() => worked.context("the download workers stopped")?,
        synced = core.clone().run_auto_sync() => synced.context("Auto-Sync stopped")?,
        () = core.clone().run_library_scan() => {}
        () = stop_signal() => tracing::info!("Kura is stopping"),
    }
    Ok(())
}

/// The database holds the mail server's password and the Jellyfin API key, which Kura has to
/// send as they are, so they cannot be stored hashed. The data folder is for Kura's own user
/// alone: `0700`, and the database files in it `0600`, also when an older Kura or the umask
/// made them readable for others. A folder Kura does not own is left as it is, with a warning.
fn keep_private(data: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let close = |path: &std::path::Path, mode: u32| {
            let Ok(metadata) = std::fs::metadata(path) else {
                return;
            };
            if metadata.permissions().mode() & 0o777 == mode {
                return;
            }
            if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            {
                tracing::warn!(
                    "could not make {} readable only for Kura ({mode:o}): {err}",
                    path.display()
                );
            }
        };
        close(data, 0o700);
        for name in ["kura.db", "kura.db-journal", "kura.db-wal", "kura.db-shm"] {
            close(&data.join(name), 0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = data;
}

/// Ctrl+C, or SIGTERM as sent by `docker stop`.
async fn stop_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                terminate.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

/// Asks the local server for its status, for Docker's HEALTHCHECK. A plain HTTP request
/// keeps an HTTP client and curl out of the image.
async fn healthcheck() -> ExitCode {
    let check = async {
        let mut address: SocketAddr = listen_address().parse()?;
        // Listening on every address means the server is reachable on loopback too.
        if address.ip().is_unspecified() {
            address.set_ip([127, 0, 0, 1].into());
        }
        let mut stream = tokio::net::TcpStream::connect(address).await?;
        let request = "GET /api/v1/status HTTP/1.0\r\nHost: localhost\r\n\r\n";
        stream.write_all(request.as_bytes()).await?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await?;
        let ok = answer.starts_with(b"HTTP/1.0 200") || answer.starts_with(b"HTTP/1.1 200");
        anyhow::ensure!(ok, "the server did not answer with 200");
        anyhow::Ok(())
    };
    match tokio::time::timeout(Duration::from_secs(5), check).await {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(err)) => {
            eprintln!("kura: unhealthy: {err:#}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("kura: unhealthy: no answer within 5 seconds");
            ExitCode::FAILURE
        }
    }
}
