use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::rpc::{build_protocol_error, PiState};

pub(super) struct ExactSession<'a> {
    id: &'a str,
    checkout: &'a Path,
    directory: PathBuf,
    path: PathBuf,
    file: File,
}

impl<'a> ExactSession<'a> {
    pub(super) fn locate(id: &'a str, checkout: &'a Path) -> io::Result<Self> {
        if !Uuid::parse_str(id).is_ok_and(|uuid| uuid.hyphenated().to_string() == id) {
            return Err(build_protocol_error("expected a full Pi session UUID"));
        }
        let directory = resolve_session_directory(checkout)?;
        let mut found = None;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(session_not_found(id, &directory));
            }
            Err(error) => return Err(error),
        };
        for entry in entries {
            let path = entry?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            let Ok((file, header)) = open_session_header(&path) else {
                continue;
            };
            if header.id != id {
                continue;
            }
            if found.is_some() {
                return Err(build_protocol_error(format!(
                    "multiple Pi files match session {id} in {}",
                    directory.display()
                )));
            }
            found = Some((path, file, header));
        }
        let (path, file, header) = found.ok_or_else(|| session_not_found(id, &directory))?;
        verify_checkout(&header.cwd, checkout)?;
        Ok(Self {
            id,
            checkout,
            directory: directory.canonicalize()?,
            path: path.canonicalize()?,
            file,
        })
    }

    pub(super) fn id(&self) -> &str {
        self.id
    }

    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(super) fn verify(&self, state: &PiState) -> io::Result<()> {
        if state.session_id != self.id {
            return Err(build_protocol_error(format!(
                "expected Pi session {}, but RPC returned {}",
                self.id, state.session_id
            )));
        }
        let path = state
            .session_file
            .as_ref()
            .filter(|path| path.is_absolute())
            .ok_or_else(|| build_protocol_error("Pi get_state did not report a session file"))?;
        if path.canonicalize()? != self.path {
            return Err(build_protocol_error("Pi opened a different session file"));
        }
        let (file, header) = open_session_header(path)?;
        let before = self.file.metadata()?;
        let after = file.metadata()?;
        if before.dev() != after.dev() || before.ino() != after.ino() || header.id != self.id {
            return Err(build_protocol_error(
                "Pi session file changed during reopen",
            ));
        }
        verify_checkout(&header.cwd, self.checkout)?;
        if state.message_count.is_none_or(|count| count == 0) {
            return Err(build_protocol_error(
                "Pi did not report reopened conversation history",
            ));
        }
        Ok(())
    }
}

fn session_not_found(id: &str, directory: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Pi session {id} not found in {}; run from the original checkout with the same Pi session store",
            directory.display()
        ),
    )
}

#[derive(Deserialize)]
struct SessionHeader {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    cwd: PathBuf,
}

fn open_session_header(path: &Path) -> io::Result<(File, SessionHeader)> {
    // Pi 1.5.2 bounds header discovery at 1 MiB. Accept only regular JSONL files
    // with a header on the first line; do not repair or create anything here.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(build_protocol_error("Pi session is not a regular file"));
    }
    let mut line = Vec::new();
    BufReader::new((&file).take(1024 * 1024)).read_until(b'\n', &mut line)?;
    if line.last() != Some(&b'\n') {
        return Err(build_protocol_error(
            "Pi session header is missing or incomplete",
        ));
    }
    let header: SessionHeader = serde_json::from_slice(&line)?;
    if header.kind != "session" {
        return Err(build_protocol_error(
            "Pi file does not begin with a session header",
        ));
    }
    Ok((file, header))
}

fn verify_checkout(saved: &Path, launch: &Path) -> io::Result<()> {
    // Pi's native open builds cwd-bound runtime services from this header.
    if !saved.is_absolute() || saved.canonicalize().as_deref().ok() != Some(launch) {
        return Err(build_protocol_error(format!(
            "Pi session belongs to checkout {}, not {}; run from the original checkout",
            saved.display(),
            launch.display()
        )));
    }
    Ok(())
}

fn resolve_session_directory(checkout: &Path) -> io::Result<PathBuf> {
    let home = env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    if let Some(directory) =
        env::var_os("PI_CODING_AGENT_SESSION_DIR").filter(|dir| !dir.is_empty())
    {
        return resolve_configured_path(Path::new(&directory), checkout, home.as_deref());
    }
    let agent_dir = match env::var_os("PI_CODING_AGENT_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => resolve_configured_path(Path::new(&dir), checkout, home.as_deref())?,
        None => require_home(home.as_deref())?.join(".pi/agent"),
    };
    for settings in [
        checkout.join(".pi/settings.json"),
        agent_dir.join("settings.json"),
    ] {
        if let Some(directory) = read_session_directory_setting(&settings)? {
            if directory.is_empty() {
                break;
            }
            return resolve_configured_path(Path::new(&directory), checkout, home.as_deref());
        }
    }
    let path = checkout
        .to_str()
        .ok_or_else(|| build_protocol_error("Pi checkout path is not UTF-8"))?;
    let slug = path.trim_start_matches('/').replace(['/', '\\', ':'], "-");
    Ok(agent_dir.join("sessions").join(format!("--{slug}--")))
}

fn read_session_directory_setting(path: &Path) -> io::Result<Option<String>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let json = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let settings: Value = serde_json::from_slice(json).map_err(|error| {
        build_protocol_error(format!(
            "cannot read Pi settings {}: {error}",
            path.display()
        ))
    })?;
    match settings.get("sessionDir") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(String::new())),
        Some(Value::String(directory)) => Ok(Some(directory.clone())),
        Some(_) => Err(build_protocol_error(format!(
            "invalid sessionDir in Pi settings {}",
            path.display()
        ))),
    }
}

fn require_home(home: Option<&Path>) -> io::Result<&Path> {
    home.ok_or_else(|| build_protocol_error("cannot locate the Pi store without HOME"))
}

fn resolve_configured_path(
    path: &Path,
    checkout: &Path,
    home: Option<&Path>,
) -> io::Result<PathBuf> {
    if let Ok(relative) = path.strip_prefix("~") {
        return Ok(require_home(home)?.join(relative));
    }
    if let Some(url) = path.to_str().filter(|path| path.starts_with("file://")) {
        return reqwest::Url::parse(url)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or_else(|| build_protocol_error("invalid file URL for Pi session storage"));
    }
    Ok(checkout.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_missing_and_explicitly_cleared_session_settings() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        assert_eq!(read_session_directory_setting(&settings).unwrap(), None);
        for (config, expected) in [
            ("{}", None),
            (r#"{"sessionDir": null}"#, Some("")),
            (r#"{"sessionDir": ""}"#, Some("")),
            ("\u{feff}{\"sessionDir\":\"sessions\"}", Some("sessions")),
        ] {
            fs::write(&settings, config).unwrap();
            assert_eq!(
                read_session_directory_setting(&settings)
                    .unwrap()
                    .as_deref(),
                expected
            );
        }
        fs::write(&settings, r#"{"sessionDir":42}"#).unwrap();
        assert!(read_session_directory_setting(&settings).is_err());
    }

    #[test]
    fn resolves_configured_pi_paths() {
        for (path, expected) in [
            ("sessions", "/checkout/sessions"),
            ("~/sessions", "/home/test/sessions"),
            ("file:///Pi%20sessions", "/Pi sessions"),
        ] {
            assert_eq!(
                resolve_configured_path(
                    Path::new(path),
                    Path::new("/checkout"),
                    Some(Path::new("/home/test"))
                )
                .unwrap(),
                Path::new(expected)
            );
        }
    }
}
