#[tokio::main]
async fn main() -> anyhow::Result<()> {
    mtpdrive_cli::run().await
}
