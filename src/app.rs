use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use crate::agent::{
    create_agent, list_agents, load_agent_content, print_agent_content_report, print_agent_list,
    print_agent_stop_report, print_created_agent_report, print_updated_agent_report, run_agent,
    stop_agents_with_systems, update_agent,
};
use crate::check::{check_waap, print_check_result};
use crate::cli::{AgentCommand, Cli, Command, TicketCommand};
use crate::git::commit_paths;
use crate::init::{init_project, print_init_report};
use crate::root::resolve_waap_root;
use crate::ticket::{
    create_ticket, get_ticket, list_tickets, print_ticket_get_report, print_ticket_list,
    print_ticket_report, print_updated_ticket_report, update_ticket,
};

/// Commit the waap state files changed by a command, then run `print` with the commit hash.
///
/// On commit failure the waap state update is left intact on disk and a non-zero exit code is
/// returned with a diagnostic on stderr.
fn commit_and_print(
    waap_root: &Path,
    paths: &[&Path],
    message: &str,
    print: impl FnOnce(&str),
) -> ExitCode {
    match commit_paths(waap_root, paths, message) {
        Ok(commit) => {
            print(&commit);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to commit waap state change: {error}");
            ExitCode::from(1)
        }
    }
}

pub(crate) fn run() -> ExitCode {
    let cli = Cli::parse();

    // `init` creates `.waap/`, so it operates on `--waap-root` (or the current directory)
    // directly rather than through `resolve_waap_root`, which requires `.waap/` to already exist.
    if matches!(cli.command, Command::Init) {
        let waap_root = cli.waap_root.clone().unwrap_or_else(|| PathBuf::from("."));
        return match init_project(&waap_root) {
            Ok(report) => commit_and_print(
                &waap_root,
                &[report.marker.as_path()],
                "waap init",
                |commit| print_init_report(&cli.output_format, &report, commit),
            ),
            Err(error) => {
                eprintln!("failed to initialize waap project: {error}");
                ExitCode::from(1)
            }
        };
    }

    let cwd = match env::current_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("failed to determine current directory: {error}");
            return ExitCode::from(1);
        }
    };
    let waap_root = match resolve_waap_root(&cwd, cli.waap_root.as_deref()) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let waap_root = &waap_root;

    match cli.command {
        Command::Init => unreachable!("handled above"),
        Command::Check => {
            let errors = check_waap(waap_root);
            print_check_result(&cli.output_format, &errors);
            if errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Command::Agent { command } => match command {
            AgentCommand::New => match create_agent(waap_root) {
                Ok(report) => commit_and_print(
                    waap_root,
                    &[report.path.as_path()],
                    &format!("waap agent new {}", report.agent_id),
                    |commit| print_created_agent_report(&cli.output_format, &report, commit),
                ),
                Err(error) => {
                    eprintln!("failed to create agent: {error}");
                    ExitCode::from(1)
                }
            },
            // `agent run` commits the running-state change from inside the attached
            // run's on_started hook, then forwards the system's exit code.
            AgentCommand::Run { agent_id, system } => {
                match run_agent(waap_root, &cli.output_format, &agent_id, &system) {
                    Ok(status) => status,
                    Err(error) => {
                        eprintln!("failed to run agent: {error}");
                        ExitCode::from(1)
                    }
                }
            }
            AgentCommand::Get { agent_id } => match load_agent_content(waap_root, &agent_id) {
                Ok((report, content)) => {
                    print_agent_content_report(&cli.output_format, &report, &content);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("failed to get agent: {error}");
                    ExitCode::from(1)
                }
            },
            AgentCommand::Stop { agent_id } => {
                match stop_agents_with_systems(waap_root, agent_id.as_deref()) {
                    Ok(reports) => {
                        if reports.is_empty() {
                            print_agent_stop_report(&cli.output_format, &reports, None);
                            return ExitCode::SUCCESS;
                        }
                        let paths: Vec<&Path> =
                            reports.iter().map(|report| report.path.as_path()).collect();
                        let ids: Vec<&str> = reports
                            .iter()
                            .map(|report| report.agent_id.as_str())
                            .collect();
                        commit_and_print(
                            waap_root,
                            &paths,
                            &format!("waap agent stop {}", ids.join(" ")),
                            |commit| {
                                print_agent_stop_report(&cli.output_format, &reports, Some(commit))
                            },
                        )
                    }
                    Err(error) => {
                        eprintln!("failed to stop agent: {error}");
                        ExitCode::from(1)
                    }
                }
            }
            AgentCommand::Update {
                agent_id,
                set_status,
                set_session_id,
            } => match update_agent(
                waap_root,
                &agent_id,
                set_status.as_ref(),
                set_session_id.as_deref(),
            ) {
                Ok(report) => commit_and_print(
                    waap_root,
                    &[report.path.as_path()],
                    &format!("waap agent update {}", report.agent_id),
                    |commit| print_updated_agent_report(&cli.output_format, &report, commit),
                ),
                Err(error) => {
                    eprintln!("failed to update agent: {error}");
                    ExitCode::from(1)
                }
            },
            AgentCommand::List { status } => match list_agents(waap_root, status.as_ref()) {
                Ok(reports) => {
                    print_agent_list(&cli.output_format, &reports);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("failed to list agents: {error}");
                    ExitCode::from(1)
                }
            },
        },
        Command::Ticket { command } => match command {
            TicketCommand::New { title, depends_on } => {
                match create_ticket(waap_root, &title, &depends_on) {
                    Ok(report) => commit_and_print(
                        waap_root,
                        &[report.path.as_path()],
                        &format!("waap ticket new {}", report.ticket_id),
                        |commit| print_ticket_report(&cli.output_format, &report, commit),
                    ),
                    Err(error) => {
                        eprintln!("failed to create ticket: {error}");
                        ExitCode::from(1)
                    }
                }
            }
            TicketCommand::Get { ticket_id } => match get_ticket(waap_root, &ticket_id) {
                Ok(report) => {
                    print_ticket_get_report(&cli.output_format, &report);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("failed to get ticket: {error}");
                    ExitCode::from(1)
                }
            },
            TicketCommand::Update {
                ticket_id,
                set_status,
                add_depends_on,
                remove_depends_on,
            } => {
                if set_status.is_none() && add_depends_on.is_empty() && remove_depends_on.is_empty()
                {
                    eprintln!("at least one of --set-status, --add-depends-on, or --remove-depends-on must be provided");
                    return ExitCode::from(1);
                }
                match update_ticket(
                    waap_root,
                    &ticket_id,
                    set_status.as_ref(),
                    &add_depends_on,
                    &remove_depends_on,
                ) {
                    Ok(report) => commit_and_print(
                        waap_root,
                        &[report.path.as_path()],
                        &format!("waap ticket update {}", report.ticket_id),
                        |commit| print_updated_ticket_report(&cli.output_format, &report, commit),
                    ),
                    Err(error) => {
                        eprintln!("failed to update ticket: {error}");
                        ExitCode::from(1)
                    }
                }
            }
            TicketCommand::List {
                status,
                blocked,
                unblocked,
            } => {
                let blocked_filter = if blocked {
                    Some(true)
                } else if unblocked {
                    Some(false)
                } else {
                    None
                };
                match list_tickets(waap_root, status.as_ref(), blocked_filter) {
                    Ok(entries) => {
                        print_ticket_list(&cli.output_format, &entries);
                        ExitCode::SUCCESS
                    }
                    Err(error) => {
                        eprintln!("failed to list tickets: {error}");
                        ExitCode::from(1)
                    }
                }
            }
        },
    }
}
