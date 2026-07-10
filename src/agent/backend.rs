use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use super::claude::ClaudeBackend;
use super::codex::CodexBackend;
use super::opencode::OpencodeBackend;
use super::AgentSystem;

pub(super) struct RunPreparation {
    pub(super) initial_session_id: Option<String>,
}

#[derive(Debug, PartialEq)]
pub(super) enum RunOutcome {
    Completed,
    Failed(ExitCode),
}

impl RunOutcome {
    pub(super) fn from_exit_status(status: ExitStatus) -> Self {
        if status.success() {
            Self::Completed
        } else {
            Self::Failed(ExitCode::from(status.code().unwrap_or(1) as u8))
        }
    }
}

pub(super) struct RunContext<'a> {
    pub(super) agent_id: &'a str,
    pub(super) prompt: &'a str,
    pub(super) initial_session_id: Option<&'a str>,
    pub(super) worktree_dir: &'a Path,
    pub(super) publish_session: &'a mut dyn FnMut(&str) -> io::Result<()>,
}

pub(super) struct AbortContext<'a> {
    pub(super) waap_root: &'a Path,
    pub(super) agent_id: &'a str,
    pub(super) session_id: &'a str,
}

pub(super) trait AgentSystemBackend {
    fn prepare_run(&mut self) -> io::Result<RunPreparation>;

    fn run(&mut self, context: RunContext<'_>) -> io::Result<RunOutcome>;

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()>;
}

pub(super) trait BackendResolver {
    fn resolve(&mut self, system: &AgentSystem) -> io::Result<&mut (dyn AgentSystemBackend + '_)>;
}

#[derive(Default)]
pub(super) struct BackendRegistry {
    opencode: Option<OpencodeBackend>,
    claude: Option<ClaudeBackend>,
    codex: Option<CodexBackend>,
}

impl BackendResolver for BackendRegistry {
    fn resolve(&mut self, system: &AgentSystem) -> io::Result<&mut (dyn AgentSystemBackend + '_)> {
        match system {
            AgentSystem::Opencode => {
                if self.opencode.is_none() {
                    self.opencode = Some(OpencodeBackend::from_env()?);
                }
                Ok(self.opencode.as_mut().expect("initialized"))
            }
            AgentSystem::Claude => {
                if self.claude.is_none() {
                    self.claude = Some(ClaudeBackend::from_env());
                }
                Ok(self.claude.as_mut().expect("initialized"))
            }
            AgentSystem::Codex => {
                if self.codex.is_none() {
                    self.codex = Some(CodexBackend::from_env());
                }
                Ok(self.codex.as_mut().expect("initialized"))
            }
        }
    }
}

#[cfg(test)]
pub(super) mod fake {
    use std::path::PathBuf;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct RunCall {
        pub(crate) agent_id: String,
        pub(crate) prompt: String,
        pub(crate) initial_session_id: Option<String>,
        pub(crate) worktree_dir: PathBuf,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct AbortCall {
        pub(crate) waap_root: PathBuf,
        pub(crate) agent_id: String,
        pub(crate) session_id: String,
    }

    pub(crate) struct FakeBackend {
        pub(crate) initial_session_id: Option<String>,
        pub(crate) late_session_id: Option<String>,
        pub(crate) outcome: Option<RunOutcome>,
        pub(crate) prepare_error: Option<String>,
        pub(crate) run_error: Option<String>,
        pub(crate) abort_error: Option<String>,
        pub(crate) prepare_calls: usize,
        pub(crate) run_calls: Vec<RunCall>,
        pub(crate) abort_calls: Vec<AbortCall>,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                initial_session_id: None,
                late_session_id: None,
                outcome: Some(RunOutcome::Completed),
                prepare_error: None,
                run_error: None,
                abort_error: None,
                prepare_calls: 0,
                run_calls: Vec::new(),
                abort_calls: Vec::new(),
            }
        }
    }

    impl AgentSystemBackend for FakeBackend {
        fn prepare_run(&mut self) -> io::Result<RunPreparation> {
            self.prepare_calls += 1;
            if let Some(error) = self.prepare_error.take() {
                return Err(io::Error::other(error));
            }
            Ok(RunPreparation {
                initial_session_id: self.initial_session_id.clone(),
            })
        }

        fn run(&mut self, context: RunContext<'_>) -> io::Result<RunOutcome> {
            self.run_calls.push(RunCall {
                agent_id: context.agent_id.to_string(),
                prompt: context.prompt.to_string(),
                initial_session_id: context.initial_session_id.map(str::to_string),
                worktree_dir: context.worktree_dir.to_path_buf(),
            });
            if let Some(error) = self.run_error.take() {
                return Err(io::Error::other(error));
            }
            if let Some(session_id) = &self.late_session_id {
                (context.publish_session)(session_id)?;
            }
            Ok(self.outcome.take().unwrap_or(RunOutcome::Completed))
        }

        fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
            self.abort_calls.push(AbortCall {
                waap_root: context.waap_root.to_path_buf(),
                agent_id: context.agent_id.to_string(),
                session_id: context.session_id.to_string(),
            });
            if let Some(error) = self.abort_error.take() {
                Err(io::Error::other(error))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct FakeResolver {
        pub(crate) opencode: FakeBackend,
        pub(crate) claude: FakeBackend,
        pub(crate) codex: FakeBackend,
        pub(crate) resolved: Vec<AgentSystem>,
        pub(crate) resolve_error: Option<AgentSystem>,
    }

    impl FakeResolver {
        pub(crate) fn backend_mut(&mut self, system: &AgentSystem) -> &mut FakeBackend {
            match system {
                AgentSystem::Opencode => &mut self.opencode,
                AgentSystem::Claude => &mut self.claude,
                AgentSystem::Codex => &mut self.codex,
            }
        }
    }

    impl BackendResolver for FakeResolver {
        fn resolve(
            &mut self,
            system: &AgentSystem,
        ) -> io::Result<&mut (dyn AgentSystemBackend + '_)> {
            self.resolved.push(system.clone());
            if self.resolve_error.as_ref() == Some(system) {
                return Err(io::Error::other("backend resolution failed"));
            }
            Ok(self.backend_mut(system))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;

    #[test]
    fn exit_status_maps_success_and_exact_failure_code() {
        assert_eq!(
            RunOutcome::from_exit_status(ExitStatus::from_raw(0)),
            RunOutcome::Completed
        );
        assert_eq!(
            RunOutcome::from_exit_status(ExitStatus::from_raw(7 << 8)),
            RunOutcome::Failed(ExitCode::from(7))
        );
        assert_eq!(
            RunOutcome::from_exit_status(ExitStatus::from_raw(9)),
            RunOutcome::Failed(ExitCode::from(1))
        );
    }

    #[test]
    fn registry_constructs_only_selected_backend_and_reuses_it() {
        let mut registry = BackendRegistry::default();

        registry.resolve(&AgentSystem::Claude).unwrap();
        registry.resolve(&AgentSystem::Claude).unwrap();

        assert!(registry.claude.is_some());
        assert!(registry.opencode.is_none());
        assert!(registry.codex.is_none());

        registry.resolve(&AgentSystem::Codex).unwrap();
        assert!(registry.codex.is_some());
        assert!(registry.opencode.is_none());
    }
}
