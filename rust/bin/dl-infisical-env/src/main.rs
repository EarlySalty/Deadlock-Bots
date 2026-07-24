use clap::Parser;
use std::{ffi::OsString, os::unix::process::CommandExt};

#[derive(Parser)]
struct Cli {
    #[arg(long, value_enum, default_value = "all")]
    profile: dl_infisical_env::Profile,
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = dl_infisical_env::config_from_env()?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base = dl_infisical_env::validated_base_url(dl_infisical_env::INFISICAL_BASE_URL)?;
    let secrets = dl_infisical_env::fetch_secrets_with_retry(&client, &base, &config).await?;
    let error = dl_infisical_env::command_with_secrets(cli.profile, &secrets, &cli.command)?.exec();
    Err(error.into())
}
