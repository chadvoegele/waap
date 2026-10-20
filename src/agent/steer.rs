use std::io;
use std::path::Path;

use serde_json::json;

use super::control::{client, socket_path};
use super::load_agent_content;
use crate::cli::OutputFormat;
use crate::root::ControlRoot;

pub(crate) fn steer_agent(
    waap_root: &Path,
    control_root: io::Result<ControlRoot>,
    agent_id: &str,
    prompt: &str,
) -> io::Result<()> {
    if prompt.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt must contain a non-whitespace character",
        ));
    }
    let (report, _) = load_agent_content(waap_root, agent_id)?;
    if report.metadata.status != "running" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "agent {agent_id} is {}, expected running",
                report.metadata.status
            ),
        ));
    }
    let path = socket_path(&control_root?, agent_id)?;
    client::submit(&path, prompt)
}

pub(crate) fn print_steering_report(format: &OutputFormat, agent_id: &str) {
    match format {
        OutputFormat::Json => println!("{}", json!({"agent_id": agent_id, "accepted": true})),
        OutputFormat::HumanReadable => println!("Steering accepted for agent {agent_id}"),
    }
}
