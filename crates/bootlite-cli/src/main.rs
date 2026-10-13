use anyhow::Result;
use bootlite_core::{flash_and_verify, list_safe_usb_drives};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "bootlite", version, about = "Fast, zero-bloat cross-platform USB image flasher")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List safe removable USB drives
    List {
        /// Include attached virtual disk images for safe testing
        #[arg(long)]
        allow_virtual: bool,
    },
    /// Flash and verify an ISO/IMG file onto a removable drive
    Flash {
        /// Path to the source .iso or .img file
        image: PathBuf,
        /// Target disk ID (e.g., disk4 on macOS, sdb on Linux, PhysicalDrive1 on Windows)
        disk_id: String,
        /// Allow flashing to a virtual test drive
        #[arg(long)]
        allow_virtual: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List { allow_virtual } => {
            let drives = list_safe_usb_drives(allow_virtual)?;
            if drives.is_empty() {
                println!("No removable USB drives detected.");
                println!("Tip: Pass --allow-virtual to see attached virtual test disks.");
                return Ok(());
            }

            println!(
                "{:<14} {:<20} {:<10} {:<14} {}",
                "ID", "RAW PATH", "SIZE", "BUS", "MODEL"
            );
            println!("{}", "-".repeat(75));
            for d in drives {
                let mb = d.size_bytes as f64 / (1024.0 * 1024.0);
                let size_str = if mb >= 1024.0 {
                    format!("{:.2} GB", mb / 1024.0)
                } else {
                    format!("{:.0} MB", mb)
                };
                println!(
                    "{:<14} {:<20} {:>10} {:<14} {}",
                    d.id,
                    d.raw_device_path.display(),
                    size_str,
                    d.bus,
                    d.model
                );
            }
        }
        Commands::Flash {
            image,
            disk_id,
            allow_virtual,
        } => {
            let clean_id = disk_id
                .trim_start_matches("/dev/r")
                .trim_start_matches("/dev/");

            let drives = list_safe_usb_drives(allow_virtual)?;
            let target = drives
                .into_iter()
                .find(|d| d.id == clean_id || d.raw_device_path.to_string_lossy() == disk_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Drive '{}' not found or blocked by safety checks! (If testing a virtual disk, pass --allow-virtual)",
                        disk_id
                    )
                })?;

            println!(
                "Flashing '{}' -> {} ({})",
                image.display(),
                target.raw_device_path.display(),
                target.model
            );

            let pb = ProgressBar::new(100);
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({msg})",
                )?
                .progress_chars("#>-"),
            );

            let hash = flash_and_verify(&image, &target, None, |p| {
                pb.set_length(p.total_bytes);
                pb.set_position(p.bytes_processed);
                pb.set_message(format!("{}: {:.1} MB/s", p.phase, p.speed_mbps));
            })?;

            pb.finish_with_message("Done!");
            println!("\nFlash & Read-Back Verification Successful!");
            println!("Verified SHA-256: {}", hash);
        }
    }

    Ok(())
}
