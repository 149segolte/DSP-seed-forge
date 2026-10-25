// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2026 149segolte <me@149segolte.dev>

use clap::{CommandFactory, Parser};
use std::num::NonZeroU32;

mod gpu_demo;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Run and verify the GPU compute demo
    #[arg(long)]
    gpu_demo: bool,

    /// Number of values processed by the GPU demo [default: 1003]
    #[arg(long, value_name = "N", requires = "gpu_demo")]
    count: Option<NonZeroU32>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.gpu_demo {
        return gpu_demo::run(cli.count.map_or(1003, NonZeroU32::get));
    }

    // Show help when no action was selected.
    Cli::command().print_help()?;
    println!();
    Ok(())
}
