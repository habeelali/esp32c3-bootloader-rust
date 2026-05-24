use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod keygen;
mod sig_block;
mod sign;
mod verify;

#[derive(Parser)]
#[command(name = "sign-image", about = "ESP32-C3 Secure Boot V2 image signing tool")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a new ECDSA P-256 signing key
    Keygen {
        /// Output path for the PKCS#8 PEM private key
        #[arg(long)]
        out: PathBuf,
        /// Optional: write Rust key constants to this path (for secure_boot_key.rs)
        #[arg(long = "out-rust")]
        out_rust: Option<PathBuf>,
    },
    /// Sign a firmware image (appends 4096-byte signature block)
    Sign {
        /// PKCS#8 PEM private key
        #[arg(long)]
        key: PathBuf,
        /// Input image binary (e.g. bootloader.bin)
        #[arg(long = "in")]
        input: PathBuf,
        /// Output signed image path
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify a signed image against a key
    Verify {
        /// PKCS#8 PEM private key (public key is derived from it)
        #[arg(long)]
        key: PathBuf,
        /// Signed image to verify
        #[arg(long = "in")]
        input: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Keygen { out, out_rust } => keygen::generate(&out, out_rust.as_deref()),
        Cmd::Sign { key, input, out } => sign::sign(&key, &input, &out),
        Cmd::Verify { key, input } => verify::verify(&key, &input),
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
