use anyhow::{Context, Result};
use bootlite_core::{
    eject_drive, flash_and_verify_with_options, inspect_image, list_safe_usb_drives,
    normalize_sha256_input, reset_and_format_drive_with_scheme, PartitionScheme, VerifyMode,
};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

fn parse_scheme(s: &str) -> Result<PartitionScheme> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(PartitionScheme::Auto),
        "gpt" => Ok(PartitionScheme::Gpt),
        "mbr" => Ok(PartitionScheme::Mbr),
        other => anyhow::bail!("Invalid partition scheme '{}' (use auto, gpt, or mbr)", other),
    }
}

fn parse_verify(s: &str) -> Result<VerifyMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "fast" => Ok(VerifyMode::Fast),
        "full" => Ok(VerifyMode::Full),
        "off" | "none" => Ok(VerifyMode::None),
        other => anyhow::bail!("Invalid verify mode '{}' (use fast, full, or off)", other),
    }
}

#[derive(Parser)]
#[command(
    name = "bootlite-cli",
    version = "0.5.1",
    about = "Raw Hardware USB Flasher, Windows 10/11 UEFI Builder & SHA-256 Verifier"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List connected removable USB / SD drives
    List {
        #[arg(long, help = "Include virtual / loopback disks")]
        virtual_disks: bool,
    },
    /// Inspect an OS image for boot signatures, partition tables, and compression
    Inspect {
        #[arg(short, long)]
        image: PathBuf,
    },
    /// Flash and hardware-verify an image to a target USB drive
    Flash {
        #[arg(short, long)]
        image: PathBuf,
        #[arg(short, long, help = "Target drive ID (e.g., disk2, sdb, PhysicalDrive1)")]
        drive: String,
        #[arg(long, default_value = "auto", help = "Partition scheme: auto, gpt, or mbr")]
        scheme: String,
        #[arg(long, default_value = "fast", help = "Verification mode: fast, full, or off")]
        verify: String,
        #[arg(long, help = "Optional expected SHA-256 hash to verify against")]
        expected_sha256: Option<String>,
        #[arg(long, help = "Automatically eject the USB drive after verification")]
        eject: bool,
        #[arg(long, help = "Allow flashing to virtual disks")]
        virtual_disks: bool,
    },
    /// Wipe MBR/GPT/Backup-GPT and format a USB drive back to full-capacity ExFAT
    Reset {
        #[arg(short, long, help = "Target drive ID (e.g., disk2, sdb, PhysicalDrive1)")]
        drive: String,
        #[arg(long, default_value = "BOOTLITE", help = "Volume label")]
        label: String,
        #[arg(long, default_value = "auto", help = "Partition scheme: auto, gpt, or mbr")]
        scheme: String,
        #[arg(long, help = "Allow resetting virtual disks")]
        virtual_disks: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List { virtual_disks } => {
            let drives = list_safe_usb_drives(virtual_disks)?;
            if drives.is_empty() {
                println!("No removable USB / SD drives detected.");
            } else {
                for d in drives {
                    let gb = d.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    println!(
                        "{:<16} {:<28} {:>7.2} GB   Bus: {:<9} Node: {}",
                        d.id,
                        d.model,
                        gb,
                        d.bus,
                        d.raw_device_path.display()
                    );
                }
            }
        }
        Commands::Inspect { image } => {
            let meta = inspect_image(&image)?;
            let mb = meta.file_size as f64 / (1024.0 * 1024.0);
            println!("File:        {}", image.display());
            println!("Size:        {:.2} MB ({} bytes)", mb, meta.file_size);
            println!("Stream Mode: {}", meta.compression);
            println!("Boot Layout: {}", meta.scheme);
            if let Some(warn) = meta.warning {
                println!("WARNING:     {}", warn);
            }
        }
        Commands::Flash {
            image,
            drive,
            scheme,
            verify,
            expected_sha256,
            eject,
            virtual_disks,
        } => {
            let parsed_scheme = parse_scheme(&scheme)?;
            let parsed_verify = parse_verify(&verify)?;
            let meta = inspect_image(&image)?;
            println!(
                "Image Layout: {} ({}) | Scheme: {} | Verify: {}",
                meta.scheme,
                meta.compression,
                parsed_scheme.label(),
                parsed_verify.label()
            );
            if let Some(warn) = &meta.warning {
                eprintln!("WARNING: {}", warn);
            }

            let drives = list_safe_usb_drives(virtual_disks)?;
            let target = drives
                .into_iter()
                .find(|d| d.id == drive)
                .with_context(|| format!("Target drive '{}' not found or locked for safety", drive))?;

            let pb = ProgressBar::new(100);
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos:>7}/{len:7} {msg}",
                )?
                .progress_chars("#>-"),
            );

            let hash = flash_and_verify_with_options(
                &image,
                &target,
                parsed_scheme,
                parsed_verify,
                None,
                |prog| {
                    pb.set_length(prog.total_bytes);
                    pb.set_position(prog.bytes_processed);
                    pb.set_message(format!("{} ({:.1} MB/s)", prog.phase, prog.speed_mbps));
                },
            )?;
            pb.finish_with_message("Verified!");

            println!("Stream SHA-256: {}", hash);
            if let Some(exp_raw) = expected_sha256 {
                if let Some(exp) = normalize_sha256_input(&exp_raw) {
                    if exp != hash {
                        anyhow::bail!(
                            "Expected SHA-256 mismatch! Expected: {}, Flashed: {}",
                            exp,
                            hash
                        );
                    }
                    println!("Expected SHA-256 matched!");
                }
            }

            if eject {
                eject_drive(&target)?;
                println!("Safely ejected {}.", target.id);
            }
        }
        Commands::Reset {
            drive,
            label,
            scheme,
            virtual_disks,
        } => {
            let parsed_scheme = parse_scheme(&scheme)?;
            let drives = list_safe_usb_drives(virtual_disks)?;
            let target = drives
                .into_iter()
                .find(|d| d.id == drive)
                .with_context(|| format!("Target drive '{}' not found or locked for safety", drive))?;
            println!(
                "Wiping partition tables and formatting {} ({}) as ExFAT [{}] ({})...",
                target.model,
                target.id,
                label,
                parsed_scheme.label()
            );
            reset_and_format_drive_with_scheme(&target, &label, parsed_scheme)?;
            println!("Drive {} successfully restored to full capacity.", target.id);
        }
    }

    Ok(())
}
