use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::cli::OutputFormat;
use crate::state::StateLock;

#[derive(Debug)]
pub(crate) struct InitReport {
    path: PathBuf,
}

pub(crate) fn init_project(state_root: &Path) -> io::Result<InitReport> {
    fs::create_dir_all(state_root)?;
    let _lock = StateLock::acquire(state_root)?;
    fs::create_dir_all(state_root.join("agents"))?;
    fs::create_dir_all(state_root.join("tickets"))?;
    Ok(InitReport {
        path: state_root.canonicalize()?,
    })
}

pub(crate) fn print_init_report(output_format: &OutputFormat, report: &InitReport) {
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            json!({
                "path": report.path.display().to_string(),
                "state_directory": report.path.display().to_string(),
            })
        ),
        OutputFormat::HumanReadable => {
            println!("Initialized waap project");
            println!("State directory: {}", report.path.display());
        }
    }
}
