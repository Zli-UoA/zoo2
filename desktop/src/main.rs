use anyhow::Result;
use zoo2_client_core::config::Config;

fn main() -> Result<()> {
    let config = Config::load_for("zoo2-desktop")?;
    zoo2_desktop::run(config);
    Ok(())
}
