use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::agent::{AgentStatus, AgentSystem, ReasoningEffort};
use crate::ticket::TicketStatus;

#[derive(Debug, Parser)]
#[command(name = "waap")]
#[command(about = "Waap Agent Automation Platform")]
pub(crate) struct Cli {
    /// Enable debug logging.
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,

    #[arg(long, value_enum, default_value = "human-readable", global = true)]
    pub(crate) output_format: OutputFormat,

    /// Override the shared state directory for the application repository.
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
    /// Create missing state directories without changing existing records.
    Init,
    /// Validate waap state.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Delivery {
    Start,
    Steer,
    Asap,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AgentCommand {
    /// Register an agent without starting a backend session.
    New {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_enum, default_value = "pi")]
        system: AgentSystem,
    },
    /// Start a turn or steer an active turn. Reads redirected stdin unless --text is supplied.
    Prompt {
        #[arg(long)]
        agent_id: String,
        #[arg(long, value_enum, default_value = "asap")]
        delivery: Delivery,
        #[arg(long)]
        text: Option<String>,

        #[arg(long, value_parser = parse_non_empty_model)]
        model: Option<String>,

        #[arg(long, value_enum)]
        reasoning_effort: Option<ReasoningEffort>,
    },
    /// Get an existing agent's record.
    Get {
        #[arg(long)]
        agent_id: String,
    },
    Stop {
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Update an existing agent's metadata.
    Update {
        #[arg(long)]
        agent_id: String,

        /// Change the display name without changing the agent ID or lifecycle.
        #[arg(long)]
        set_name: String,
    },
    /// List existing agent ids.
    List {
        #[arg(long, value_enum)]
        status: Option<AgentStatus>,
    },
}

fn parse_non_empty_model(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("model must not be empty".to_string())
    } else {
        Ok(value.to_string())
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum TicketCommand {
    /// Create a new ticket from stdin.
    New {
        #[arg(long)]
        name: Option<String>,

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

    use clap::{Parser, ValueEnum};

    use super::{AgentCommand, Cli, Command, Delivery, TicketCommand};
    use crate::agent::{AgentStatus, AgentSystem, ReasoningEffort};
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
    fn parses_global_verbose_flag() {
        let cli = Cli::try_parse_from(["waap", "check", "-v"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn verbose_defaults_to_false() {
        let cli = Cli::try_parse_from(["waap", "check"]).unwrap();
        assert!(!cli.verbose);
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
    fn parses_failed_agent_status_for_list() {
        let list = Cli::try_parse_from(["waap", "agent", "list", "--status", "failed"]).unwrap();
        assert!(matches!(
            list.command,
            Command::Agent {
                command: AgentCommand::List {
                    status: Some(AgentStatus::Failed)
                }
            }
        ));
    }

    #[test]
    fn omitted_and_explicit_asap_parse_identically() {
        for extra in [None, Some("--delivery=asap")] {
            let args = [
                "waap",
                "--output-format",
                "json",
                "agent",
                "prompt",
                "--agent-id",
                "aa-test",
            ]
            .into_iter()
            .chain(extra);
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(matches!(cli.output_format, OutputFormat::Json));
            assert!(
                matches!(cli.command, Command::Agent { command: AgentCommand::Prompt { agent_id, delivery: Delivery::Asap, text: None, .. } } if agent_id == "aa-test")
            );
        }
    }

    #[test]
    fn parses_explicit_new_system() {
        for (name, expected) in [
            ("claude", AgentSystem::Claude),
            ("codex", AgentSystem::Codex),
            ("opencode", AgentSystem::Opencode),
            ("pi", AgentSystem::Pi),
        ] {
            let cli = Cli::try_parse_from(["waap", "agent", "new", "--system", name]).unwrap();
            assert!(
                matches!(cli.command, Command::Agent { command: AgentCommand::New { name: None, system } } if system == expected)
            );
        }
    }

    #[test]
    fn parses_agent_prompt_with_codex_model_and_reasoning_effort() {
        let cli = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--agent-id",
            "aa-3881fda0",
            "--delivery=start",
            "--model",
            "gpt-5.4",
            "--reasoning-effort",
            "high",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Prompt {
                    model: Some(model),
                    reasoning_effort: Some(ReasoningEffort::High),
                    ..
                }
            } if model == "gpt-5.4"
        ));
    }

    #[test]
    fn parses_codex_start_options_independently() {
        let model = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--agent-id",
            "aa-3881fda0",
            "--model",
            "gpt-5.4",
        ])
        .unwrap();
        let effort = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--agent-id",
            "aa-3881fda0",
            "--reasoning-effort",
            "ultra",
        ])
        .unwrap();

        assert!(matches!(
            model.command,
            Command::Agent {
                command: AgentCommand::Prompt {
                    model: Some(model),
                    reasoning_effort: None,
                    ..
                }
            } if model == "gpt-5.4"
        ));
        assert!(matches!(
            effort.command,
            Command::Agent {
                command: AgentCommand::Prompt {
                    model: None,
                    reasoning_effort: Some(ReasoningEffort::Ultra),
                    ..
                }
            }
        ));
    }

    #[test]
    fn parses_all_reasoning_efforts_before_backend_validation() {
        for effort in ReasoningEffort::value_variants() {
            let cli = Cli::try_parse_from([
                "waap",
                "agent",
                "prompt",
                "--agent-id",
                "aa-3881fda0",
                "--reasoning-effort",
                effort.as_str(),
            ])
            .unwrap();
            assert!(matches!(
                cli.command,
                Command::Agent {
                    command: AgentCommand::Prompt {
                        reasoning_effort: Some(parsed),
                        ..
                    }
                } if parsed == *effort
            ));
        }
    }

    #[test]
    fn agent_prompt_rejects_invalid_effort_and_empty_model() {
        let invalid_effort = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--agent-id",
            "aa-3881fda0",
            "--reasoning-effort",
            "extreme",
        ])
        .unwrap_err();
        let empty_model = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--agent-id",
            "aa-3881fda0",
            "--model",
            "",
        ])
        .unwrap_err();

        assert_eq!(invalid_effort.kind(), clap::error::ErrorKind::InvalidValue);
        assert_eq!(empty_model.kind(), clap::error::ErrorKind::ValueValidation);
        assert!(empty_model.to_string().contains("model must not be empty"));
    }

    #[test]
    fn new_rejects_invalid_system_argument() {
        let error =
            Cli::try_parse_from(["waap", "agent", "new", "--system", "cursor"]).unwrap_err();
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
    fn prompt_requires_id_and_preserves_explicit_text() {
        let error = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--delivery=steer",
            "--text",
            "text",
        ])
        .unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        let prompt = "  雪\n\"quoted\" '$HOME' `echo data`  ";
        let cli = Cli::try_parse_from([
            "waap",
            "agent",
            "prompt",
            "--delivery=steer",
            "--agent-id",
            "aa-test",
            "--text",
            prompt,
        ])
        .unwrap();
        assert!(
            matches!(cli.command, Command::Agent { command: AgentCommand::Prompt { agent_id, delivery: Delivery::Steer, text: Some(parsed), .. } } if agent_id == "aa-test" && parsed == prompt)
        );
    }

    #[test]
    fn prompt_rejects_future_options_and_backend_override() {
        use clap::error::ErrorKind;
        for (option, error) in [
            ("--delivery=queue", ErrorKind::InvalidValue),
            ("--image=img.png", ErrorKind::UnknownArgument),
            ("--system=codex", ErrorKind::UnknownArgument),
        ] {
            assert_eq!(
                Cli::try_parse_from(["waap", "agent", "prompt", "--agent-id", "aa-test", option])
                    .unwrap_err()
                    .kind(),
                error
            );
        }
        for retired in ["run", "steer"] {
            assert_eq!(
                Cli::try_parse_from(["waap", "agent", retired])
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidSubcommand
            );
        }
    }

    #[test]
    fn parses_agent_new_arguments() {
        let cli = Cli::try_parse_from(["waap", "agent", "new"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::New {
                    name: None,
                    system: AgentSystem::Pi
                }
            }
        ));
    }

    #[test]
    fn parses_agent_new_name_argument() {
        let cli = Cli::try_parse_from(["waap", "agent", "new", "--name", "Custom Agent"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::New { name: Some(name), system: AgentSystem::Pi }
            } if name == "Custom Agent"
        ));
    }

    #[test]
    fn parses_ticket_new_with_optional_name() {
        let unnamed = Cli::try_parse_from(["waap", "ticket", "new"]).unwrap();
        let named = Cli::try_parse_from(["waap", "ticket", "new", "--name", "New Ticket"]).unwrap();

        assert!(matches!(
            unnamed.command,
            Command::Ticket {
                command: TicketCommand::New {
                    name: None,
                    depends_on
                }
            } if depends_on.is_empty()
        ));
        assert!(matches!(
            named.command,
            Command::Ticket {
                command: TicketCommand::New {
                    name: Some(name),
                    depends_on
                }
            } if name == "New Ticket" && depends_on.is_empty()
        ));
    }

    #[test]
    fn agent_new_rejects_role_argument() {
        let error =
            Cli::try_parse_from(["waap", "agent", "new", "--role", "developer"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn agent_prompt_requires_agent_id_argument() {
        let error = Cli::try_parse_from(["waap", "agent", "prompt"]).unwrap_err();

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
    fn parses_metadata_only_update() {
        let cli = Cli::try_parse_from([
            "waap",
            "--output-format",
            "json",
            "agent",
            "update",
            "--agent-id",
            "aa-test",
            "--set-name",
            "Renamed",
        ])
        .unwrap();
        assert!(matches!(cli.output_format, OutputFormat::Json));
        assert!(
            matches!(cli.command, Command::Agent { command: AgentCommand::Update { agent_id, set_name } } if agent_id == "aa-test" && set_name == "Renamed")
        );
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
    fn agent_update_rejects_lifecycle_arguments() {
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

        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }
}
