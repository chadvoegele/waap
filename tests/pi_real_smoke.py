#!/usr/bin/env python3
import argparse
from dataclasses import dataclass, field
from functools import partial
import http.server
import json
import logging
import os
from pathlib import Path
import shlex
import signal
import subprocess
import tempfile
import threading
import tomllib
import uuid


@dataclass
class ProviderState:
    mode: str = "complete"
    ready: threading.Event = field(default_factory=threading.Event)
    release: threading.Event = field(default_factory=threading.Event)
    requests: list = field(default_factory=list)

    def reset(self, mode):
        self.mode = mode
        self.ready.clear()
        self.release.clear()
        self.requests.clear()


class Provider(http.server.BaseHTTPRequestHandler):
    def __init__(self, *args, state, **kwargs):
        self.state = state
        super().__init__(*args, **kwargs)

    def log_message(self, *_):
        pass

    def send_chunk(self, delta, finish=None):
        record = {"id": "smoke", "object": "chat.completion.chunk", "model": "fixture",
                  "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
        self.wfile.write(("data: " + json.dumps(record) + "\n\n").encode())
        self.wfile.flush()

    def send_tool_call(self, command):
        self.send_chunk({"tool_calls": [{
            "index": 0, "id": "smoke-tool", "type": "function",
            "function": {"name": "bash", "arguments": json.dumps({"command": command})}
        }]})
        self.send_chunk({}, "tool_calls")

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.state.requests.append(request)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        try:
            self.send_chunk({"role": "assistant"})
            if self.state.mode == "stop":
                self.send_chunk({"content": "waiting for stop"})
                self.state.ready.set()
                self.state.release.wait(15)
                self.send_chunk({}, "stop")
            elif self.state.mode == "complete":
                if len(self.state.requests) == 1:
                    self.state.ready.set()
                    assert self.state.release.wait(15)
                    self.send_tool_call("printf 'real Pi tool execution\\n' > smoke.txt && git add smoke.txt "
                                        "&& git -c commit.gpgsign=false commit -m 'Pi smoke'")
                else:
                    self.send_chunk({"content": "smoke complete"})
                    self.send_chunk({}, "stop")
            elif len(self.state.requests) == 1:
                recalled = find_sentinel(request["messages"])
                self.send_tool_call(f"pwd -P > resume-cwd.txt; printf '%s' {shlex.quote(recalled)} > recalled.txt")
            else:
                self.send_chunk({"content": find_sentinel(request["messages"])})
                self.send_chunk({}, "stop")
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


def find_sentinel(messages):
    for message in messages:
        if message["role"] != "user":
            continue
        content = message["content"]
        text = content if isinstance(content, str) else " ".join(part.get("text", "") for part in content)
        marker = "Remember this smoke sentinel: "
        if marker in text:
            return text.split(marker, 1)[1].split()[0]
    return "history-not-reopened"


def run(args, *, cwd, env, text=None, error=None):
    result = subprocess.run(args, cwd=cwd, env=env, input=text, text=True,
                            capture_output=True, timeout=25)
    if error is None:
        assert result.returncode == 0, (args, result.stdout, result.stderr)
    else:
        assert result.returncode != 0 and error in result.stderr, (args, result.stdout, result.stderr)
    return result.stdout


def write_pi_configuration(config, port, shutdown_notification):
    config.mkdir()
    if shutdown_notification:
        (config / "extensions").mkdir()
        (config / "extensions/shutdown.ts").write_text('''import { writeFileSync } from "node:fs";
export default function(pi) {
  pi.on("session_shutdown", async (_event, ctx) => {
    ctx.ui.notify("shutdown-notification" + "x".repeat(128 * 1024));
    writeFileSync(process.env.PI_CODING_AGENT_DIR + "/shutdown-ran", "yes");
  });
}
''')
    (config / "models.json").write_text(json.dumps({"providers": {"local-smoke": {
        "baseUrl": f"http://127.0.0.1:{port}/v1", "api": "openai-completions",
        "apiKey": "local-fixture", "models": [{"id": "fixture", "reasoning": False,
                                            "contextWindow": 128000, "maxTokens": 4096}]
    }}}))
    (config / "settings.json").write_text(json.dumps({
        "retry": {"enabled": False}, "enableInstallTelemetry": False
    }))


def build_environment(config, runtime, pi):
    return dict(PATH=os.environ.get("PATH", os.defpath), HOME=str(config.parent), LANG="C.UTF-8",
               XDG_CONFIG_HOME=str(config.parent / "config"), XDG_DATA_HOME=str(config.parent / "data"),
               PI_CODING_AGENT_DIR=str(config), PI_OFFLINE="1",
               WAAP_PI_BIN=pi, XDG_RUNTIME_DIR=str(runtime),
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
               GIT_CONFIG_COUNT="2", GIT_CONFIG_KEY_0="commit.gpgsign", GIT_CONFIG_VALUE_0="false",
               GIT_CONFIG_KEY_1="core.hooksPath", GIT_CONFIG_VALUE_1="/dev/null",
               GIT_AUTHOR_NAME="Smoke", GIT_AUTHOR_EMAIL="smoke@example.com",
               GIT_COMMITTER_NAME="Smoke", GIT_COMMITTER_EMAIL="smoke@example.com")


def stop_owner(owner):
    try:
        os.killpg(owner.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        owner.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(owner.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        owner.communicate(timeout=5)
    finally:
        try:
            os.killpg(owner.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def read_agent(state, agent):
    record = state / "agents" / agent / "agent.md"
    return tomllib.loads(record.read_text().split("+++", 2)[1])


def session_files(env):
    default_store = Path(env["PI_CODING_AGENT_DIR"]) / "sessions"
    return set(Path(env.get("PI_CODING_AGENT_SESSION_DIR", default_store)).rglob("*.jsonl"))


def verify_run(repo, state, env, agent, provider):
    config = Path(env["PI_CODING_AGENT_DIR"])
    if (config / "extensions/shutdown.ts").exists():
        assert (config / "shutdown-ran").read_text() == "yes"
    status = "completed" if provider.mode == "complete" else "aborted"
    record = read_agent(state, agent)
    assert record["status"] == status and record["system"] == "pi", record
    assert not list(Path(env["XDG_RUNTIME_DIR"]).rglob("*.sock"))
    assert session_files(env), "Pi did not persist the real session"
    if provider.mode == "complete":
        assert "real Pi tool execution" in run(["git", "show", "HEAD:smoke.txt"], cwd=repo, env=env)
        assert any("Include snow 雪 in your review" in json.dumps(request, ensure_ascii=False)
                   for request in provider.requests[1:]), provider.requests
    logging.info("real Pi %s: %s, session persisted, socket cleaned", provider.mode, status)
    return record


def run_smoke_case(base, checkout, other, state, env, provider):
    config = Path(env["PI_CODING_AGENT_DIR"])
    (config / "shutdown-ran").unlink(missing_ok=True)
    nested = checkout / "nested"
    nested.mkdir(exist_ok=True)
    sentinel = uuid.uuid4().hex
    agent = json.loads(run(base + ["--output-format", "json", "agent", "new", "--name", provider.mode],
                           cwd=nested, env=env))["agent_id"]
    prompt = base + ["agent", "prompt", "--delivery=start", "--agent-id", agent,
                     "--model", "local-smoke/fixture"]
    owner = subprocess.Popen(prompt + ["--text", f"Remember this smoke sentinel: {sentinel}"],
                             cwd=nested, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, start_new_session=True)
    try:
        assert provider.ready.wait(15), "real Pi did not contact the local provider"
        if provider.mode == "complete":
            run(base + ["agent", "prompt", "--delivery=steer", "--agent-id", agent,
                        "--text", "Include snow 雪 in your review"], cwd=nested, env=env)
            provider.release.set()
        else:
            run(base + ["agent", "stop", "--agent-id", agent], cwd=nested, env=env)
        out, err = owner.communicate(timeout=15)
        assert owner.returncode == 0, (out, err)
        assert "shutdown-notification" not in out + err, (out, err)
        first = verify_run(checkout, state, env, agent, provider)
    finally:
        provider.release.set()
        stop_owner(owner)

    files = session_files(env)
    session, = (path for path in files if json.loads(path.read_bytes().splitlines()[0])["id"] == first["session_id"])
    original = session.read_bytes()
    provider.reset("resume")
    followup = prompt + ["--text", "Recall the earlier smoke sentinel and check your tool directory."]
    run(followup, cwd=other, env=env, error="run from the original checkout")
    assert not provider.requests and session.read_bytes() == original
    assert read_agent(state, agent) == first
    assert session_files(env) == files

    (config / "shutdown-ran").unlink(missing_ok=True)
    assert sentinel in run(followup, cwd=nested, env=env)
    if (config / "extensions/shutdown.ts").exists():
        assert (config / "shutdown-ran").read_text() == "yes"
    assert len(provider.requests) == 2, provider.requests
    assert (checkout / "recalled.txt").read_text() == sentinel
    assert Path((checkout / "resume-cwd.txt").read_text().strip()) == checkout.resolve()
    assert not (other / "resume-cwd.txt").exists()
    completed = read_agent(state, agent)
    assert completed == {**first, "status": "completed"}
    assert session_files(env) == files
    assert session.read_bytes().startswith(original)
    assert not list(Path(env["XDG_RUNTIME_DIR"]).rglob("*.sock"))

    session.unlink()
    provider.reset("resume")
    run(followup, cwd=nested, env=env, error="not found")
    assert not provider.requests and read_agent(state, agent) == completed
    assert session_files(env) == files - {session}
    logging.info("real Pi resume after %s: recalled sentinel in original checkout; cross-checkout and missing history rejected",
                 first["status"])


def main():
    parser = argparse.ArgumentParser(description="Run real Pi against an isolated local SSE provider.")
    parser.add_argument("waap", type=Path)
    parser.add_argument("--pi", default="pi")
    parser.add_argument("--shutdown-notification", action="store_true",
                        help="emit a 128 KiB UI notification after native stdin EOF")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    provider = ProviderState()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), partial(Provider, state=provider))
    server.daemon_threads = False
    server_thread = threading.Thread(target=server.serve_forever)
    server_thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="waap-pi-smoke-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            runtime = root / "run"
            runtime.mkdir(mode=0o700)
            config = root / "pi"
            write_pi_configuration(config, server.server_port, args.shutdown_notification)
            env = build_environment(config, runtime, args.pi)
            run(["git", "init", "-q", "--initial-branch=main"], cwd=repo, env=env)
            run(["git", "commit", "--allow-empty", "-qm", "seed"], cwd=repo, env=env)
            checkout, other = root / "linked", root / "other"
            run(["git", "worktree", "add", "-qb", "smoke", str(checkout)], cwd=repo, env=env)
            run(["git", "worktree", "add", "-q", "--detach", str(other)], cwd=repo, env=env)
            state = root / "state"
            base = [str(args.waap.resolve()), "--waap-root", str(state)]
            run(base + ["init"], cwd=repo, env=env)
            provider.reset("complete")
            run_smoke_case(base, checkout, other, state, env, provider)
            provider.reset("stop")
            env["PI_CODING_AGENT_SESSION_DIR"] = str(root / "shared Pi sessions")
            run_smoke_case(base, checkout, other, state, env, provider)
    finally:
        provider.release.set()
        server.shutdown()
        server.server_close()
        server_thread.join()


if __name__ == "__main__":
    main()
