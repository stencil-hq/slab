//! `slab-tui` command-line entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    slab_tui::cli::main(std::env::args().skip(1))
}
