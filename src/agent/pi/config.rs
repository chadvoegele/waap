use crate::agent::{AgentRunOptions, ReasoningEffort};
use clap::ValueEnum;
use std::env;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

pub(super) const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SCRUBBED_PI_ENV: [&str; 5] = [
    "PI_SESSION_ID",
    "PI_SESSION_FILE",
    "PI_PROVIDER",
    "PI_MODEL",
    "PI_REASONING_LEVEL",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PiRunConfig {
    executable: OsString,
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    pub(super) command_timeout: Duration,
}

impl Default for PiRunConfig {
    fn default() -> Self {
        Self {
            executable: OsString::from("pi"),
            model: None,
            reasoning_effort: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

impl PiRunConfig {
    pub(super) fn from_env(options: &AgentRunOptions) -> io::Result<Self> {
        let executable = env::var_os("WAAP_PI_BIN").unwrap_or_else(|| OsString::from("pi"));
        if executable.is_empty() {
            return Err(invalid_input("WAAP_PI_BIN must not be empty"));
        }

        let model = match &options.model {
            Some(model) => Some(model.clone()),
            None => optional_non_empty_env("WAAP_PI_MODEL")?,
        };
        let reasoning_effort = match options.reasoning_effort {
            Some(effort) => Some(validate_pi_effort(effort, "--reasoning-effort")?),
            None => optional_pi_effort_env()?,
        };

        Ok(Self {
            executable,
            model,
            reasoning_effort,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        })
    }
}

fn optional_non_empty_env(name: &str) -> io::Result<Option<String>> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => {
            Err(invalid_input(format!("{name} must not be empty")))
        }
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(invalid_input(format!("{name} must be valid UTF-8")))
        }
    }
}

fn optional_pi_effort_env() -> io::Result<Option<ReasoningEffort>> {
    let Some(value) = optional_non_empty_env("WAAP_PI_REASONING_EFFORT")? else {
        return Ok(None);
    };
    let effort = ReasoningEffort::parse(&value).ok_or_else(|| {
        invalid_input(format!(
            "invalid WAAP_PI_REASONING_EFFORT {value:?}; accepted values: {}",
            pi_effort_labels().join(", ")
        ))
    })?;
    validate_pi_effort(effort, "WAAP_PI_REASONING_EFFORT").map(Some)
}

fn validate_pi_effort(effort: ReasoningEffort, source: &str) -> io::Result<ReasoningEffort> {
    if effort == ReasoningEffort::Ultra {
        return Err(invalid_input(format!(
            "{source} does not support ultra for Pi; accepted values: {}",
            pi_effort_labels().join(", ")
        )));
    }
    Ok(effort)
}

fn pi_effort_labels() -> Vec<&'static str> {
    ReasoningEffort::value_variants()
        .iter()
        .copied()
        .filter(|effort| *effort != ReasoningEffort::Ultra)
        .map(ReasoningEffort::as_str)
        .collect()
}

fn pi_thinking(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "off",
        _ => effort.as_str(),
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(super) fn pi_command(config: &PiRunConfig, agent_id: &str, worktree_dir: &Path) -> Command {
    let mut command = Command::new(&config.executable);
    command
        .arg("--mode")
        .arg("rpc")
        .arg("--approve")
        .arg("--name")
        .arg(format!("waap {agent_id}"))
        .current_dir(worktree_dir);
    if let Some(model) = &config.model {
        command.arg("--model").arg(model);
    }
    if let Some(effort) = config.reasoning_effort {
        command.arg("--thinking").arg(pi_thinking(effort));
    }
    for name in SCRUBBED_PI_ENV {
        command.env_remove(name);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::PI_ENV_LOCK;
    #[test]
    fn config_uses_cli_precedence_maps_none_and_rejects_ultra() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let previous = [
            env::var_os("WAAP_PI_BIN"),
            env::var_os("WAAP_PI_MODEL"),
            env::var_os("WAAP_PI_REASONING_EFFORT"),
        ];
        env::set_var("WAAP_PI_BIN", "/tmp/pi-bin");
        env::set_var("WAAP_PI_MODEL", "env-model");
        env::set_var("WAAP_PI_REASONING_EFFORT", "low");

        let fallback = PiRunConfig::from_env(&AgentRunOptions::default()).unwrap();
        let overridden = PiRunConfig::from_env(&AgentRunOptions {
            model: Some("cli-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::None),
        })
        .unwrap();
        assert_eq!(fallback.model.as_deref(), Some("env-model"));
        assert_eq!(fallback.executable, OsString::from("/tmp/pi-bin"));
        assert_eq!(fallback.reasoning_effort, Some(ReasoningEffort::Low));
        assert_eq!(overridden.model.as_deref(), Some("cli-model"));
        assert_eq!(pi_thinking(overridden.reasoning_effort.unwrap()), "off");
        for effort in ReasoningEffort::value_variants()
            .iter()
            .copied()
            .filter(|effort| !matches!(effort, ReasoningEffort::None | ReasoningEffort::Ultra))
        {
            assert_eq!(pi_thinking(effort), effort.as_str());
        }
        assert!(PiRunConfig::from_env(&AgentRunOptions {
            model: None,
            reasoning_effort: Some(ReasoningEffort::Ultra),
        })
        .unwrap_err()
        .to_string()
        .contains("does not support ultra"));

        for (name, value) in [
            ("WAAP_PI_BIN", previous[0].clone()),
            ("WAAP_PI_MODEL", previous[1].clone()),
            ("WAAP_PI_REASONING_EFFORT", previous[2].clone()),
        ] {
            match value {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
        }
    }

    #[test]
    fn config_rejects_invalid_environment_values_unless_cli_overrides_them() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let previous_model = env::var_os("WAAP_PI_MODEL");
        let previous_effort = env::var_os("WAAP_PI_REASONING_EFFORT");
        env::set_var("WAAP_PI_MODEL", "  ");
        assert!(PiRunConfig::from_env(&AgentRunOptions::default()).is_err());
        env::set_var("WAAP_PI_MODEL", "valid");
        env::set_var("WAAP_PI_REASONING_EFFORT", "off");
        assert!(PiRunConfig::from_env(&AgentRunOptions::default()).is_err());
        assert!(PiRunConfig::from_env(&AgentRunOptions {
            model: Some("cli".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        })
        .is_ok());
        match previous_model {
            Some(value) => env::set_var("WAAP_PI_MODEL", value),
            None => env::remove_var("WAAP_PI_MODEL"),
        }
        match previous_effort {
            Some(value) => env::set_var("WAAP_PI_REASONING_EFFORT", value),
            None => env::remove_var("WAAP_PI_REASONING_EFFORT"),
        }
    }

    #[test]
    fn command_has_rpc_approval_session_name_options_cwd_and_scrubbed_metadata() {
        let config = PiRunConfig {
            model: Some("openai/gpt".to_string()),
            reasoning_effort: Some(ReasoningEffort::Max),
            ..PiRunConfig::default()
        };
        let dir = Path::new("/repo/with space");
        let command = pi_command(&config, "aa-12345678", dir);

        assert_eq!(command.as_std().get_program(), "pi");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            [
                "--mode",
                "rpc",
                "--approve",
                "--name",
                "waap aa-12345678",
                "--model",
                "openai/gpt",
                "--thinking",
                "max",
            ]
        );
        assert_eq!(command.as_std().get_current_dir(), Some(dir));
        let removed = command
            .as_std()
            .get_envs()
            .filter_map(|(name, value)| value.is_none().then_some(name.to_string_lossy()))
            .collect::<Vec<_>>();
        for name in SCRUBBED_PI_ENV {
            assert!(removed.iter().any(|removed| removed == name));
        }
    }
}
