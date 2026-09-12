mod capture;
mod replay;
mod report;
mod rpc;
mod types;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::{Parser, Subcommand};
use std::{
    fs,
    path::{Path, PathBuf},
};
use types::{ComparisonReport, Fixture};

#[derive(Parser)]
#[command(
    name = "verify-mainnet-upgrade",
    about = "Capture and replay Manifest mainnet market instructions against two program builds"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Capture a slot range, then replay it against the deployed and candidate programs.
    Run {
        #[arg(long)]
        market: String,
        #[arg(long)]
        new_program: PathBuf,
        #[arg(long)]
        old_program: Option<PathBuf>,
        #[arg(
            long,
            env = "SOLANA_RPC_URL",
            default_value = "https://api.mainnet-beta.solana.com"
        )]
        rpc_url: String,
        #[arg(long, default_value = "finalized")]
        commitment: String,
        #[arg(long, default_value_t = 1)]
        slots: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Capture a reusable fixture and the currently deployed Manifest program.
    Capture {
        #[arg(long)]
        market: String,
        #[arg(
            long,
            env = "SOLANA_RPC_URL",
            default_value = "https://api.mainnet-beta.solana.com"
        )]
        rpc_url: String,
        #[arg(long, default_value = "finalized")]
        commitment: String,
        #[arg(long, default_value_t = 1)]
        slots: u64,
        #[arg(long)]
        output: PathBuf,
    },
    /// Replay an existing fixture without making RPC requests.
    Replay {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long)]
        new_program: PathBuf,
        #[arg(long)]
        old_program: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "error");
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            market,
            new_program,
            old_program,
            rpc_url,
            commitment,
            slots,
            output,
        } => {
            validate_slots(slots)?;
            println!("Capturing market {market} for at least {slots} slot(s)...");
            let (fixture, deployed_program) =
                capture::capture(rpc_url, commitment, market, slots).await?;
            let output = output.unwrap_or_else(|| {
                PathBuf::from(format!(
                    "manifest-replay-{}-{}-{}",
                    &fixture.market[..8],
                    fixture.start_slot,
                    fixture.end_slot
                ))
            });
            save_capture(&output, &fixture, &deployed_program)?;
            let old_path = old_program.unwrap_or_else(|| output.join("deployed-program.so"));
            run_replay(
                &output.join("fixture.json"),
                &fixture,
                &old_path,
                &new_program,
                &output,
            )
            .await?;
        }
        Command::Capture {
            market,
            rpc_url,
            commitment,
            slots,
            output,
        } => {
            validate_slots(slots)?;
            println!("Capturing market {market} for at least {slots} slot(s)...");
            let (fixture, deployed_program) =
                capture::capture(rpc_url, commitment, market, slots).await?;
            save_capture(&output, &fixture, &deployed_program)?;
            println!(
                "Captured {} Manifest instructions from {} market-touching transactions in slots {}..={} in {}",
                fixture.instructions.len(),
                fixture.transactions_touching_market,
                fixture.start_slot,
                fixture.end_slot,
                output.display()
            );
        }
        Command::Replay {
            fixture: fixture_path,
            new_program,
            old_program,
            output,
        } => {
            let fixture: Fixture = serde_json::from_slice(
                &fs::read(&fixture_path)
                    .with_context(|| format!("could not read {}", fixture_path.display()))?,
            )?;
            let capture_dir = fixture_path.parent().unwrap_or_else(|| Path::new("."));
            let old_path = old_program.unwrap_or_else(|| capture_dir.join("deployed-program.so"));
            let output = output.unwrap_or_else(|| capture_dir.join("replay"));
            run_replay(&fixture_path, &fixture, &old_path, &new_program, &output).await?;
        }
    }
    Ok(())
}

fn validate_slots(slots: u64) -> Result<()> {
    if slots == 0 {
        bail!("--slots must be at least 1");
    }
    Ok(())
}

fn save_capture(output: &Path, fixture: &Fixture, deployed_program: &[u8]) -> Result<()> {
    fs::create_dir_all(output)?;
    fs::write(
        output.join("fixture.json"),
        serde_json::to_vec_pretty(fixture)?,
    )?;
    fs::write(output.join("deployed-program.so"), deployed_program)?;
    fs::write(
        output.join("chain-final-market.bin"),
        BASE64.decode(&fixture.chain_final_market.data_base64)?,
    )?;
    println!("Saved capture to {}", output.display());
    Ok(())
}

async fn run_replay(
    fixture_path: &Path,
    fixture: &Fixture,
    old_path: &Path,
    new_path: &Path,
    output: &Path,
) -> Result<()> {
    fs::create_dir_all(output)?;
    println!(
        "Replaying {} instructions against old program...",
        fixture.instructions.len()
    );
    let old = replay::replay(fixture, "old", old_path).await?;
    println!(
        "Replaying {} instructions against new program...",
        fixture.instructions.len()
    );
    let new = replay::replay(fixture, "new", new_path).await?;
    let report: ComparisonReport =
        report::build_report(fixture_path.display().to_string(), fixture, old, new)?;
    fs::write(
        output.join("old-final-market.bin"),
        BASE64.decode(&report.old.final_market_data_base64)?,
    )?;
    fs::write(
        output.join("new-final-market.bin"),
        BASE64.decode(&report.new.final_market_data_base64)?,
    )?;
    save_final_accounts(&output.join("old-final-accounts"), &report.old)?;
    save_final_accounts(&output.join("new-final-accounts"), &report.new)?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    report::print_report(&report);
    println!(
        "\nFull report and final account bytes: {}",
        output.display()
    );
    Ok(())
}

fn save_final_accounts(directory: &Path, replay: &types::ReplayResult) -> Result<()> {
    fs::create_dir_all(directory)?;
    for (address, account) in &replay.final_accounts {
        fs::write(directory.join(format!("{address}.bin")), &account.data)?;
    }
    Ok(())
}
