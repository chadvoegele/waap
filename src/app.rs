use std::env;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use env_logger::{Builder, Env, Target};
use log::LevelFilter;

use crate::agent::{
    create_agent_in_context, list_agents, load_agent_content, print_agent_content_report,
    print_agent_list, print_agent_stop_report, print_created_agent_report,
    print_updated_agent_report, run_agent_in_context, stop_agents_with_systems_in_context,
    update_agent_in_context,
};
use crate::check::{check_central_state, print_central_check_result, print_check_errors};
use crate::cli::{AgentCommand, Cli, Command, OutputFormat, TicketCommand};
use crate::git::StateStore;
use crate::init::{init_project, print_init_report};
use crate::repair::{print_repair_report, repair_project};
use crate::root::{resolve_project_context, ProjectContext, StateRootRequirement};
use crate::ticket::{
    create_ticket_in_context, get_ticket, list_tickets, print_ticket_get_report, print_ticket_list,
    print_ticket_report, print_updated_ticket_report, update_ticket_in_context,
};

fn command_error(context: &str, error: io::Error) -> ExitCode {
    eprintln!("{context}: {error}");
    ExitCode::from(1)
}

fn init_logging(verbose: bool) {
    let mut builder = if verbose {
        let mut builder = Builder::new();
        builder.filter_level(LevelFilter::Debug);
        builder
    } else {
        Builder::from_env(Env::new().filter_or("WAAP_LOG_LEVEL", "error"))
    };
    builder.target(Target::Stderr).init();
}

pub(crate) fn run() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    let cwd = match env::current_dir() {
        Ok(dir) => dir,
        Err(error) => return command_error("failed to determine current directory", error),
    };

    run_command(cli, &cwd)
}

fn run_command(cli: Cli, cwd: &Path) -> ExitCode {
    let requirement = match &cli.command {
        Command::Init | Command::Repair | Command::Check => StateRootRequirement::MayBeMissing,
        Command::Agent { .. } | Command::Ticket { .. } => StateRootRequirement::Existing,
    };
    let context = match resolve_project_context(cwd, cli.waap_root.as_deref(), requirement) {
        Ok(context) => context,
        Err(error) => return command_error("failed to resolve waap project", error),
    };
    log::debug!("resolved state directory: {}", context.state_root.display());

    match cli.command {
        Command::Init => run_init(&context, cli.waap_root.is_some(), &cli.output_format),
        Command::Repair => run_repair(&context, cli.waap_root.is_some(), &cli.output_format),
        Command::Check => run_check(&context, cli.waap_root.is_some(), &cli.output_format),
        Command::Agent { command } => {
            if let Some(exit) =
                validate_state(&context, cli.waap_root.is_some(), &cli.output_format)
            {
                return exit;
            }
            run_agent_command(command, &context, &cli.output_format)
        }
        Command::Ticket { command } => {
            if let Some(exit) =
                validate_state(&context, cli.waap_root.is_some(), &cli.output_format)
            {
                return exit;
            }
            run_ticket_command(command, &context, &cli.output_format)
        }
    }
}

fn run_init(
    context: &ProjectContext,
    explicit_state_root: bool,
    output_format: &OutputFormat,
) -> ExitCode {
    match init_project(context, explicit_state_root) {
        Ok(report) => {
            print_init_report(output_format, &report);
            ExitCode::SUCCESS
        }
        Err(error) => command_error("failed to initialize waap project", error),
    }
}

fn run_repair(
    context: &ProjectContext,
    explicit_state_root: bool,
    output_format: &OutputFormat,
) -> ExitCode {
    match repair_project(context, explicit_state_root) {
        Ok(report) => {
            print_repair_report(output_format, &report);
            ExitCode::SUCCESS
        }
        Err(error) => command_error("failed to repair waap state", error),
    }
}

fn run_check(
    context: &ProjectContext,
    explicit_state_root: bool,
    output_format: &OutputFormat,
) -> ExitCode {
    let report = check_central_state(context, explicit_state_root);
    print_warnings(&report.warnings);
    print_central_check_result(output_format, &report);
    if report.errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn validate_state(
    context: &ProjectContext,
    explicit_state_root: bool,
    output_format: &OutputFormat,
) -> Option<ExitCode> {
    let report = check_central_state(context, explicit_state_root);
    print_warnings(&report.warnings);
    if report.errors.is_empty() {
        None
    } else {
        print_check_errors(output_format, &report.errors);
        Some(ExitCode::from(1))
    }
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("WARNING: {warning}");
    }
}

fn run_agent_command(
    command: AgentCommand,
    context: &ProjectContext,
    output_format: &OutputFormat,
) -> ExitCode {
    let state_root = &context.state_root;
    let store = StateStore::from_project_context(context);
    match command {
        AgentCommand::New { name } => match create_agent_in_context(store, name.as_deref()) {
            Ok(report) => {
                print_created_agent_report(output_format, &report);
                ExitCode::SUCCESS
            }
            Err(error) => command_error("failed to create agent", error),
        },
        AgentCommand::Run { agent_id, system } => match context.application_source_root() {
            Ok(_) => match run_agent_in_context(store, output_format, &agent_id, &system) {
                Ok(status) => status,
                Err(error) => command_error("failed to run agent", error),
            },
            Err(error) => command_error("failed to run agent", error),
        },
        AgentCommand::Get { agent_id } => match load_agent_content(state_root, &agent_id) {
            Ok((report, content)) => {
                print_agent_content_report(output_format, &report, &content);
                ExitCode::SUCCESS
            }
            Err(error) => command_error("failed to get agent", error),
        },
        AgentCommand::Stop { agent_id } => {
            match stop_agents_with_systems_in_context(store, agent_id.as_deref()) {
                Ok(report) => {
                    print_agent_stop_report(output_format, &report);
                    ExitCode::SUCCESS
                }
                Err(error) => command_error("failed to stop agent", error),
            }
        }
        AgentCommand::Update {
            agent_id,
            set_status,
            set_session_id,
        } => match update_agent_in_context(
            store,
            &agent_id,
            set_status.as_ref(),
            set_session_id.as_deref(),
        ) {
            Ok(report) => {
                print_updated_agent_report(output_format, &report);
                ExitCode::SUCCESS
            }
            Err(error) => command_error("failed to update agent", error),
        },
        AgentCommand::List { status } => match list_agents(state_root, status.as_ref()) {
            Ok(reports) => {
                print_agent_list(output_format, &reports);
                ExitCode::SUCCESS
            }
            Err(error) => command_error("failed to list agents", error),
        },
    }
}

fn run_ticket_command(
    command: TicketCommand,
    context: &ProjectContext,
    output_format: &OutputFormat,
) -> ExitCode {
    let state_root = &context.state_root;
    let store = StateStore::from_project_context(context);
    match command {
        TicketCommand::New { name, depends_on } => {
            match create_ticket_in_context(store, name.as_deref(), &depends_on) {
                Ok(report) => {
                    print_ticket_report(output_format, &report);
                    ExitCode::SUCCESS
                }
                Err(error) => command_error("failed to create ticket", error),
            }
        }
        TicketCommand::Get { ticket_id } => match get_ticket(state_root, &ticket_id) {
            Ok(report) => {
                print_ticket_get_report(output_format, &report);
                ExitCode::SUCCESS
            }
            Err(error) => command_error("failed to get ticket", error),
        },
        TicketCommand::Update {
            ticket_id,
            set_status,
            add_depends_on,
            remove_depends_on,
        } => {
            if set_status.is_none() && add_depends_on.is_empty() && remove_depends_on.is_empty() {
                eprintln!("at least one of --set-status, --add-depends-on, or --remove-depends-on must be provided");
                return ExitCode::from(1);
            }
            match update_ticket_in_context(
                store,
                &ticket_id,
                set_status.as_ref(),
                &add_depends_on,
                &remove_depends_on,
            ) {
                Ok(report) => {
                    print_updated_ticket_report(output_format, &report);
                    ExitCode::SUCCESS
                }
                Err(error) => command_error("failed to update ticket", error),
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
            match list_tickets(state_root, status.as_ref(), blocked_filter) {
                Ok(entries) => {
                    print_ticket_list(output_format, &entries);
                    ExitCode::SUCCESS
                }
                Err(error) => command_error("failed to list tickets", error),
            }
        }
    }
}
