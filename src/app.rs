use std::env;
use std::process::ExitCode;

use clap::Parser;

use crate::agent::{
    create_agent, list_agents, load_agent_content, print_agent_content_report, print_agent_list,
    print_agent_stop_report, print_created_agent_report, print_updated_agent_report, run_agent,
    stop_agents_with_systems, update_agent,
};
use crate::check::{check_waap, print_check_errors, print_check_result};
use crate::cli::{AgentCommand, Cli, Command, TicketCommand};
use crate::init::{init_project, print_init_report};
use crate::mutation::MutationError;
use crate::root::resolve_waap_root;
use crate::ticket::{
    create_ticket, get_ticket, list_tickets, print_ticket_get_report, print_ticket_list,
    print_ticket_report, print_updated_ticket_report, update_ticket,
};

fn mutation_error(context: &str, error: MutationError) -> ExitCode {
    match error {
        MutationError::Operation(error) => eprintln!("{context}: {error}"),
        MutationError::Commit(error) => {
            eprintln!("failed to commit waap state change: {error}");
        }
    }
    ExitCode::from(1)
}

pub(crate) fn run() -> ExitCode {
    let cli = Cli::parse();

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

    if matches!(&cli.command, Command::Agent { .. } | Command::Ticket { .. }) {
        let errors = check_waap(waap_root);
        if !errors.is_empty() {
            print_check_errors(&cli.output_format, &errors);
            return ExitCode::from(1);
        }
    }

    match cli.command {
        Command::Init => match init_project(waap_root) {
            Ok(report) => {
                print_init_report(&cli.output_format, &report);
                ExitCode::SUCCESS
            }
            Err(error) => mutation_error("failed to initialize waap project", error),
        },
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
            AgentCommand::New { name } => match create_agent(waap_root, name.as_deref()) {
                Ok(report) => {
                    print_created_agent_report(&cli.output_format, &report);
                    ExitCode::SUCCESS
                }
                Err(error) => mutation_error("failed to create agent", error),
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
                    Ok(report) => {
                        print_agent_stop_report(&cli.output_format, &report);
                        ExitCode::SUCCESS
                    }
                    Err(error) => mutation_error("failed to stop agent", error),
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
                Ok(report) => {
                    print_updated_agent_report(&cli.output_format, &report);
                    ExitCode::SUCCESS
                }
                Err(error) => mutation_error("failed to update agent", error),
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
            TicketCommand::New { name, depends_on } => {
                match create_ticket(waap_root, name.as_deref(), &depends_on) {
                    Ok(report) => {
                        print_ticket_report(&cli.output_format, &report);
                        ExitCode::SUCCESS
                    }
                    Err(error) => mutation_error("failed to create ticket", error),
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
                    Ok(report) => {
                        print_updated_ticket_report(&cli.output_format, &report);
                        ExitCode::SUCCESS
                    }
                    Err(error) => mutation_error("failed to update ticket", error),
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
