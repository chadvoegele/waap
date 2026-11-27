use std::io;

use serde_json::json;

use super::control::{client::ControlClient, SocketPath};
use crate::cli::OutputFormat;
use crate::root::ControlRoot;

pub(super) fn steer_agent(
    control_root: io::Result<ControlRoot>,
    agent_id: &str,
    prompt: &str,
) -> io::Result<()> {
    let path = SocketPath::for_agent(&control_root?, agent_id)?;
    super::runtime::new()?.block_on(async {
        ControlClient::connect(path.as_path())
            .await?
            .steer(prompt)
            .await
    })
}

pub(crate) fn print_steering_report(format: &OutputFormat, agent_id: &str) {
    match format {
        OutputFormat::Json => println!("{}", json!({"agent_id": agent_id, "accepted": true})),
        OutputFormat::HumanReadable => println!("Steering accepted for agent {agent_id}"),
    }
}
