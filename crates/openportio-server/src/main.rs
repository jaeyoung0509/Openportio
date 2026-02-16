use openportio_server::observability;
use openportio_server::OpenportioServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _observability = observability::init_observability_from_env()
        .map_err(|err| format!("failed to initialize observability: {err}"))?;

    OpenportioServer::new().run().await?;
    Ok(())
}
