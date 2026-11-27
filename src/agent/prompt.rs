use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use super::backend::SessionOpen;
use super::{
    read_agent_record, transition_agent_status, write_agent_record, AgentRunOptions, AgentStatus,
    AgentSystem,
};
use crate::cli::{Delivery, OutputFormat};
use crate::root::ControlRoot;

pub(crate) fn read_prompt(
    text: Option<String>,
    terminal: bool,
    mut stdin: impl Read,
) -> io::Result<String> {
    let text = match text {
        Some(text) => text,
        None if terminal => return Err(invalid("supply --text or redirect prompt input")),
        None => {
            let mut text = String::new();
            stdin.read_to_string(&mut text)?;
            text
        }
    };
    if text.trim().is_empty() {
        return Err(invalid("prompt must contain a non-whitespace character"));
    }
    Ok(text)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(crate) struct PromptRequest {
    pub agent_id: String,
    pub delivery: Delivery,
    pub text: String,
    pub options: AgentRunOptions,
}

pub(super) struct StartPlan {
    pub agent_id: String,
    pub prompt: String,
    pub system: AgentSystem,
    pub prior: AgentStatus,
    pub open: SessionOpen,
}

pub(super) enum Route<B> {
    Steer,
    Start(StartPlan, B),
}

pub(super) fn decide_and_claim<B>(
    root: &Path,
    request: &PromptRequest,
    configure: impl FnOnce(&AgentSystem, &AgentRunOptions) -> io::Result<B>,
) -> io::Result<Route<B>> {
    let _lock = crate::state::StateLock::acquire(root)?;
    let id = &request.agent_id;
    let (mut metadata, body) = read_agent_record(root, id)?;
    let prior = AgentStatus::parse(&metadata.status).expect("validated status");
    if prior == AgentStatus::Running && request.delivery != Delivery::Start {
        if request.options != AgentRunOptions::default() {
            return Err(invalid(
                "model and reasoning effort apply only when starting a turn",
            ));
        }
        return Ok(Route::Steer);
    }
    if request.delivery == Delivery::Steer {
        return Err(invalid(format!(
            "agent {id} is {}, expected running",
            prior.as_str()
        )));
    }
    transition_agent_status(&mut metadata, AgentStatus::Running)?;
    let open = match (prior, &metadata.session_id) {
        (AgentStatus::Ready, None) => SessionOpen::Fresh,
        (AgentStatus::Ready, Some(id)) => {
            return Err(invalid(format!("ready agent already has session id {id}")))
        }
        (_, Some(saved)) if !saved.trim().is_empty() => SessionOpen::ResumeExact(saved.clone()),
        _ => {
            return Err(invalid(format!(
                "agent {id} has no saved session to resume"
            )))
        }
    };
    let backend = configure(&metadata.system, &request.options)?;
    write_agent_record(root, id, &metadata, &body)?;
    Ok(Route::Start(
        StartPlan {
            agent_id: id.clone(),
            prompt: request.text.clone(),
            system: metadata.system,
            prior,
            open,
        },
        backend,
    ))
}

pub(crate) fn prompt_agent(
    repository_root: &Path,
    root: &Path,
    control: impl FnOnce() -> io::Result<ControlRoot>,
    output: &OutputFormat,
    request: PromptRequest,
) -> io::Result<ExitCode> {
    match decide_and_claim(root, &request, |system, options| {
        system.backend(Some(options))
    })? {
        Route::Steer => {
            super::steer::steer_agent(control(), &request.agent_id, &request.text)?;
            super::steer::print_steering_report(output, &request.agent_id);
            Ok(ExitCode::SUCCESS)
        }
        Route::Start(plan, mut backend) => super::run::own_turn(
            repository_root,
            root,
            control(),
            output,
            &plan,
            &mut backend,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_is_validated_without_trimming_or_falling_back() {
        let input = "  # 雪\n\n`'$HOME'`\n";
        assert_eq!(read_prompt(None, false, input.as_bytes()).unwrap(), input);
        assert_eq!(
            read_prompt(Some(input.into()), true, &[][..]).unwrap(),
            input
        );
        assert!(read_prompt(None, true, input.as_bytes()).is_err());
        assert!(read_prompt(None, false, &[][..]).is_err());
        assert!(read_prompt(Some("  \n".into()), false, input.as_bytes()).is_err());
    }

    const ID: &str = "aa-route";
    const TEXT: &str = "  # Follow-up 雪\n";

    fn seed(root: &Path, status: AgentStatus, session: Option<&str>) {
        write_agent_record(
            root,
            ID,
            &super::super::AgentMetadata {
                name: None,
                creation_date: "2026-06-18T15:00:34Z".into(),
                status: status.as_str().into(),
                session_id: session.map(str::to_owned),
                system: AgentSystem::Codex,
            },
            "",
        )
        .unwrap();
    }

    fn request(delivery: Delivery) -> PromptRequest {
        PromptRequest {
            agent_id: ID.into(),
            delivery,
            text: TEXT.into(),
            options: AgentRunOptions::default(),
        }
    }

    #[test]
    fn delivery_matrix_changes_state_only_for_starts() {
        use AgentStatus::*;
        use Delivery::*;
        for (prior, routes) in [
            (Ready, ["start", "reject", "start"]),
            (Running, ["reject", "steer", "steer"]),
            (Completed, ["start", "reject", "start"]),
            (Aborted, ["start", "reject", "start"]),
            (Failed, ["reject", "reject", "reject"]),
        ] {
            for (delivery, expect) in [Start, Steer, Asap].into_iter().zip(routes) {
                let dir = tempfile::tempdir().unwrap();
                let saved = (prior != Ready).then_some(" exact ID ");
                seed(dir.path(), prior, saved);
                let path = super::super::agent_path(dir.path(), ID);
                let before = std::fs::read(&path).unwrap();
                let route = decide_and_claim(dir.path(), &request(delivery), |system, options| {
                    assert_eq!(expect, "start");
                    assert_eq!(*system, AgentSystem::Codex);
                    assert_eq!(*options, AgentRunOptions::default());
                    Ok(())
                });
                match route {
                    Ok(Route::Start(plan, ())) => {
                        assert_eq!(expect, "start");
                        assert_eq!(plan.prior, prior);
                        assert_eq!(plan.prompt, TEXT);
                        match (plan.open, saved) {
                            (SessionOpen::Fresh, None) => (),
                            (SessionOpen::ResumeExact(actual), Some(expected)) => {
                                assert_eq!(actual, expected)
                            }
                            _ => panic!("wrong session intent"),
                        }
                        let (record, body) = read_agent_record(dir.path(), ID).unwrap();
                        assert_eq!(record.status, "running");
                        assert_eq!(record.session_id.as_deref(), saved);
                        assert_eq!(record.system, AgentSystem::Codex);
                        assert!(body.is_empty());
                    }
                    Ok(Route::Steer) => assert_eq!(expect, "steer"),
                    Err(error) => assert_eq!(expect, "reject", "{error}"),
                }
                if expect != "start" {
                    assert_eq!(std::fs::read(path).unwrap(), before);
                }
            }
        }
    }

    #[test]
    fn invalid_identity_or_steering_options_do_not_claim_or_configure() {
        let dir = tempfile::tempdir().unwrap();
        let path = super::super::agent_path(dir.path(), ID);
        let try_claim = |request| {
            let before = std::fs::read(&path).unwrap();
            let result = decide_and_claim(dir.path(), &request, |_, _| -> io::Result<()> {
                panic!("must not configure")
            });
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        };
        for (status, session) in [
            (AgentStatus::Ready, Some("old")),
            (AgentStatus::Completed, None),
            (AgentStatus::Aborted, Some(" \t")),
        ] {
            seed(dir.path(), status, session);
            try_claim(request(Delivery::Asap));
        }
        seed(dir.path(), AgentStatus::Running, None);
        let mut steer = request(Delivery::Asap);
        steer.options.model = Some("test-model".into());
        try_claim(steer);
        seed(dir.path(), AgentStatus::Ready, None);
        let contents = std::fs::read_to_string(&path)
            .unwrap()
            .replace("system = \"codex\"\n", "");
        std::fs::write(&path, contents).unwrap();
        try_claim(request(Delivery::Start));
    }

    #[test]
    fn simultaneous_starts_configure_and_claim_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed(root, AgentStatus::Ready, None);
        let gate = std::sync::Barrier::new(2);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let start = || {
            gate.wait();
            decide_and_claim(root, &request(Delivery::Start), |_, _| {
                assert_eq!(calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed), 0);
                Ok(())
            })
        };
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(start);
            let b = start();
            (a.join().unwrap(), b)
        });
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(read_agent_record(root, ID).unwrap().0.status, "running");
    }

    #[test]
    fn selects_the_replacement_socket_after_releasing_the_route_lock() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        use std::time::Duration;

        let repo = tempfile::tempdir().unwrap();
        crate::test_git::init_repo_with_commit(repo.path());
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        seed(root.path(), AgentStatus::Running, Some("saved"));
        let control =
            crate::root::find_control_root_in(repo.path(), Some(runtime.path().as_os_str()))
                .unwrap();
        let socket = super::super::control::SocketPath::for_agent(&control, ID).unwrap();
        let path = socket.as_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old = UnixListener::bind(path).unwrap();
        let mut receiver = None;
        let code = prompt_agent(repo.path(), root.path(), || {
            let lock = crate::state::StateLock::acquire(root.path())?;
            std::fs::remove_file(path)?;
            let replacement = UnixListener::bind(path)?;
            drop(lock);
            let state = root.path().to_owned();
            receiver = Some(std::thread::spawn(move || {
                let (mut stream, _) = replacement.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut frame = String::new();
                BufReader::new(&stream).read_line(&mut frame).unwrap();
                let request: serde_json::Value = serde_json::from_str(&frame).unwrap();
                assert_eq!(request["prompt"], TEXT);
                let _lock = crate::state::StateLock::acquire(&state).unwrap();
                writeln!(stream, "{}", serde_json::json!({"version":1,"request_id":request["request_id"],"accepted":true})).unwrap();
            }));
            Ok(control)
        }, &OutputFormat::Json, request(Delivery::Asap)).unwrap();
        receiver.unwrap().join().unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        old.set_nonblocking(true).unwrap();
        assert_eq!(old.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            read_agent_record(root.path(), ID).unwrap().0.status,
            "running"
        );
    }
}
