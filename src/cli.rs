use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::agent::{AgentStatus, AgentSystem};
use crate::ticket::TicketStatus;

#[derive(Debug, Parser)]
#[command(name = "waap")]
#[command(about = "Waap Agent Automation Platform")]
pub(crate) struct Cli {
    #[arg(long, value_enum, default_value = "human-readable", global = true)]
    pub(crate) output_format: OutputFormat,

    /// Waap project root: the directory containing `.waap/`. When omitted, the nearest ancestor
    /// `.waap/` is used, bounded by the git root.
    #[arg(long, global = true)]
    pub(crate) waap_root: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum OutputFormat {
    Json,
    HumanReadable,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Initialize a new waap project.
    Init,
    /// Validate .waap state.
    Check,
    /// Manage agents.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Manage tickets.
    Ticket {
        #[command(subcommand)]
        command: TicketCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AgentCommand {
    /// Create a new agent from stdin.
    New {
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Run an existing agent with the selected agent system.
    Run {
        #[arg(long)]
        agent_id: String,

        /// Agent system used to run the agent.
        #[arg(long, value_enum, default_value = "opencode")]
        system: AgentSystem,
    },
    /// Get an existing agent's metadata and markdown content.
    Get {
        #[arg(long)]
        agent_id: String,
    },
    /// Stop running agents, aborting OpenCode sessions when session_id is present.
    Stop {
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Update an existing agent's metadata.
    Update {
        #[arg(long)]
        agent_id: String,

        #[arg(long, value_enum)]
        set_status: Option<AgentStatus>,

        #[arg(long)]
        set_session_id: Option<String>,
    },
    /// List existing agent ids.
    List {
        #[arg(long, value_enum)]
        status: Option<AgentStatus>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum TicketCommand {
    /// Create a new ticket from stdin.
    New {
        #[arg(long)]
        title: String,

        #[arg(long)]
        depends_on: Vec<String>,
    },
    /// Get an existing ticket.
    Get {
        #[arg(long)]
        ticket_id: String,
    },
    /// Update an existing ticket.
    Update {
        #[arg(long)]
        ticket_id: String,
        #[arg(long, value_enum)]
        set_status: Option<TicketStatus>,
        #[arg(long)]
        add_depends_on: Vec<String>,
        #[arg(long)]
        remove_depends_on: Vec<String>,
    },
    /// List existing ticket ids.
    List {
        #[arg(long, value_enum)]
        status: Option<TicketStatus>,
        #[arg(long, conflicts_with = "unblocked")]
        blocked: bool,
        #[arg(long, conflicts_with = "blocked")]
        unblocked: bool,
    },
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{AgentCommand, Cli, Command, TicketCommand};
    use crate::agent::{AgentStatus, AgentSystem};
    use crate::cli::OutputFormat;
    use crate::ticket::TicketStatus;

    #[test]
    fn parses_init_command() {
        let cli = Cli::try_parse_from(["waap", "--waap-root", "/some/path", "init"]).unwrap();

        assert_eq!(cli.waap_root, Some(PathBuf::from("/some/path")));
        assert!(matches!(cli.command, Command::Init));
    }

    #[test]
    fn parses_waap_root_argument() {
        let cli = Cli::try_parse_from(["waap", "--waap-root", "/some/path", "check"]).unwrap();
        assert_eq!(cli.waap_root, Some(PathBuf::from("/some/path")));
    }

    #[test]
    fn waap_root_defaults_to_none() {
        let cli = Cli::try_parse_from(["waap", "check"]).unwrap();
        assert_eq!(cli.waap_root, None);
    }

    #[test]
    fn rejects_old_waap_root_flag() {
        let result = Cli::try_parse_from(["waap", "--repo-root", "/some/path", "check"]);
        assert!(result.is_err());
    }

    #[test]
    fn parses_ticket_get_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "ticket",
            "get",
            "--ticket-id",
            "tt-new-ticket",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::Get { ticket_id }
            } if ticket_id == "tt-new-ticket"
        ));
    }

    #[test]
    fn parses_ticket_update_set_status() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "ticket",
            "update",
            "--ticket-id",
            "tt-new-ticket",
            "--set-status",
            "in-progress",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::Update {
                    ticket_id,
                    set_status: Some(TicketStatus::InProgress),
                    ..
                }
            } if ticket_id == "tt-new-ticket"
        ));
    }

    #[test]
    fn parses_ticket_update_add_depends_on() {
        let cli = Cli::try_parse_from([
            "waap",
            "ticket",
            "update",
            "--ticket-id",
            "tt-new-ticket",
            "--add-depends-on",
            "tt-dep-a",
            "--add-depends-on",
            "tt-dep-b",
        ])
        .unwrap();

        if let Command::Ticket {
            command:
                TicketCommand::Update {
                    ticket_id,
                    set_status,
                    add_depends_on,
                    remove_depends_on,
                },
        } = cli.command
        {
            assert_eq!(ticket_id, "tt-new-ticket");
            assert!(set_status.is_none());
            assert_eq!(add_depends_on, vec!["tt-dep-a", "tt-dep-b"]);
            assert!(remove_depends_on.is_empty());
        } else {
            panic!("unexpected command");
        }
    }

    #[test]
    fn parses_ticket_update_remove_depends_on() {
        let cli = Cli::try_parse_from([
            "waap",
            "ticket",
            "update",
            "--ticket-id",
            "tt-new-ticket",
            "--remove-depends-on",
            "tt-dep-a",
        ])
        .unwrap();

        if let Command::Ticket {
            command:
                TicketCommand::Update {
                    ticket_id,
                    set_status,
                    add_depends_on,
                    remove_depends_on,
                },
        } = cli.command
        {
            assert_eq!(ticket_id, "tt-new-ticket");
            assert!(set_status.is_none());
            assert!(add_depends_on.is_empty());
            assert_eq!(remove_depends_on, vec!["tt-dep-a"]);
        } else {
            panic!("unexpected command");
        }
    }

    #[test]
    fn ticket_update_rejects_invalid_status_argument() {
        let error = Cli::try_parse_from([
            "waap",
            "ticket",
            "update",
            "--ticket-id",
            "tt-new-ticket",
            "--set-status",
            "ready",
        ])
        .unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn parses_ticket_list_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "ticket",
            "list",
            "--status",
            "in-progress",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::List {
                    status: Some(TicketStatus::InProgress),
                    blocked: false,
                    unblocked: false,
                }
            }
        ));
    }

    #[test]
    fn ticket_list_rejects_invalid_status_arguments() {
        let error =
            Cli::try_parse_from(["waap", "ticket", "list", "--status", "ready"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn parses_ticket_list_blocked_argument() {
        let cli = Cli::try_parse_from(["waap", "ticket", "list", "--blocked"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::List {
                    status: None,
                    blocked: true,
                    unblocked: false,
                }
            }
        ));
    }

    #[test]
    fn parses_ticket_list_unblocked_argument() {
        let cli = Cli::try_parse_from(["waap", "ticket", "list", "--unblocked"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::List {
                    status: None,
                    blocked: false,
                    unblocked: true,
                }
            }
        ));
    }

    #[test]
    fn ticket_list_rejects_both_blocked_and_unblocked() {
        let error = Cli::try_parse_from(["waap", "ticket", "list", "--blocked", "--unblocked"])
            .unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn parses_agent_list_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "agent",
            "list",
            "--status",
            "running",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::List {
                    status: Some(AgentStatus::Running)
                }
            }
        ));
    }

    #[test]
    fn agent_list_rejects_invalid_status_arguments() {
        let error =
            Cli::try_parse_from(["waap", "agent", "list", "--status", "in-progress"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn parses_agent_run_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "agent",
            "run",
            "--agent-id",
            "aa-3881fda0",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Run {
                    agent_id,
                    system: AgentSystem::Opencode,
                }
            } if agent_id == "aa-3881fda0"
        ));
    }

    #[test]
    fn parses_agent_run_system_argument() {
        let cli = Cli::try_parse_from([
            "waap",
            "agent",
            "run",
            "--agent-id",
            "aa-3881fda0",
            "--system",
            "claude",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Run {
                    agent_id,
                    system: AgentSystem::Claude,
                }
            } if agent_id == "aa-3881fda0"
        ));
    }

    #[test]
    fn parses_agent_run_with_codex_system() {
        let cli = Cli::try_parse_from([
            "waap",
            "agent",
            "run",
            "--agent-id",
            "aa-3881fda0",
            "--system",
            "codex",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Run {
                    agent_id,
                    system: AgentSystem::Codex,
                }
            } if agent_id == "aa-3881fda0"
        ));
    }

    #[test]
    fn agent_run_rejects_invalid_system_argument() {
        let error = Cli::try_parse_from([
            "waap",
            "agent",
            "run",
            "--agent-id",
            "aa-3881fda0",
            "--system",
            "cursor",
        ])
        .unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn parses_agent_get_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "agent",
            "get",
            "--agent-id",
            "aa-3881fda0",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Get { agent_id }
            } if agent_id == "aa-3881fda0"
        ));
    }

    #[test]
    fn agent_get_requires_agent_id_argument() {
        let error = Cli::try_parse_from(["waap", "agent", "get"]).unwrap_err();

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn parses_agent_new_arguments() {
        let cli = Cli::try_parse_from(["waap", "agent", "new"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::New { agent_id: None }
            }
        ));
    }

    #[test]
    fn parses_agent_new_agent_id_argument() {
        let cli =
            Cli::try_parse_from(["waap", "agent", "new", "--agent-id", "custom_agent"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::New { agent_id: Some(agent_id) }
            } if agent_id == "custom_agent"
        ));
    }

    #[test]
    fn agent_new_rejects_role_argument() {
        let error =
            Cli::try_parse_from(["waap", "agent", "new", "--role", "developer"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn agent_run_requires_agent_id_argument() {
        let error = Cli::try_parse_from(["waap", "agent", "run"]).unwrap_err();

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn parses_agent_stop_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "agent",
            "stop",
            "--agent-id",
            "aa-3881fda0",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Stop {
                    agent_id: Some(agent_id)
                }
            } if agent_id == "aa-3881fda0"
        ));
    }

    #[test]
    fn parses_agent_stop_without_agent_id() {
        let cli = Cli::try_parse_from(["waap", "agent", "stop"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Stop { agent_id: None }
            }
        ));
    }

    #[test]
    fn parses_agent_update_arguments() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "agent",
            "update",
            "--agent-id",
            "aa-3881fda0",
            "--set-status",
            "running",
            "--set-session-id",
            "ses_123",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Update {
                    agent_id,
                    set_status: Some(AgentStatus::Running),
                    set_session_id: Some(session_id),
                }
            } if agent_id == "aa-3881fda0" && session_id == "ses_123"
        ));
    }

    #[test]
    fn parses_output_format_after_ticket_list_flags() {
        let cli = Cli::try_parse_from([
            "waap",
            "ticket",
            "list",
            "--status",
            "pending",
            "--unblocked",
            "--output-format",
            "json",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::List {
                    status: Some(TicketStatus::Pending),
                    blocked: false,
                    unblocked: true,
                }
            }
        ));
    }

    #[test]
    fn parses_waap_root_after_ticket_list_flags() {
        let cli = Cli::try_parse_from([
            "waap",
            "ticket",
            "list",
            "--status",
            "pending",
            "--waap-root",
            "/some/path",
        ])
        .unwrap();

        assert_eq!(cli.waap_root, Some(PathBuf::from("/some/path")));
        assert!(matches!(
            cli.command,
            Command::Ticket {
                command: TicketCommand::List {
                    status: Some(TicketStatus::Pending),
                    blocked: false,
                    unblocked: false,
                }
            }
        ));
    }

    #[test]
    fn parses_output_format_after_agent_list_flags() {
        let cli = Cli::try_parse_from([
            "waap",
            "agent",
            "list",
            "--status",
            "running",
            "--output-format",
            "json",
        ])
        .unwrap();

        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::List {
                    status: Some(AgentStatus::Running)
                }
            }
        ));
    }

    #[test]
    fn parses_waap_root_after_agent_get_flags() {
        let cli = Cli::try_parse_from([
            "waap",
            "agent",
            "get",
            "--agent-id",
            "aa-3881fda0",
            "--waap-root",
            "/some/path",
        ])
        .unwrap();

        assert_eq!(cli.waap_root, Some(PathBuf::from("/some/path")));
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Get { agent_id }
            } if agent_id == "aa-3881fda0"
        ));
    }

    #[test]
    fn agent_update_rejects_invalid_status_arguments() {
        let error = Cli::try_parse_from([
            "waap",
            "agent",
            "update",
            "--agent-id",
            "aa-3881fda0",
            "--set-status",
            "pending",
        ])
        .unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }
}
