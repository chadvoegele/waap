#!/usr/bin/env python3
"""Opt-in smoke with real installed Pi and a deterministic local SSE provider.

Usage: python3 tests/pi_real_smoke.py /absolute/path/to/waap [--pi /path/to/pi]
No external provider, credentials, or user Pi configuration are used.
"""

import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading


def run(args, *, cwd, env, text=None):
    result = subprocess.run(args, cwd=cwd, env=env, input=text, text=True,
                            capture_output=True, timeout=20)
    assert result.returncode == 0, (args, result.stdout, result.stderr)
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("waap", type=Path)
    parser.add_argument("--pi", default="pi")
    args = parser.parse_args()
    waap = str(args.waap.resolve())
    ready = threading.Event()
    release = threading.Event()
    requests = []
    mode = "complete"

    class Provider(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.append(request)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()

            def chunk(delta, finish=None):
                record = {"id": "smoke", "object": "chat.completion.chunk", "model": "fixture",
                          "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
                self.wfile.write(("data: " + json.dumps(record) + "\n\n").encode())
                self.wfile.flush()

            try:
                chunk({"role": "assistant"})
                if mode == "stop":
                    chunk({"content": "waiting for stop"})
                    ready.set()
                    release.wait(15)
                    chunk({}, "stop")
                elif len(requests) == 1:
                    ready.set()
                    assert release.wait(15)
                    command = "printf 'real Pi tool execution\\n' > smoke.txt && git add smoke.txt && git -c commit.gpgsign=false commit -m 'Pi smoke'"
                    chunk({"tool_calls": [{"index": 0, "id": "smoke-tool", "type": "function",
                                           "function": {"name": "bash", "arguments": json.dumps({"command": command})}}]})
                    chunk({}, "tool_calls")
                else:
                    chunk({"content": "smoke complete"})
                    chunk({}, "stop")
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="waap-pi-smoke-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            runtime = root / "run"
            runtime.mkdir(mode=0o700)
            config = root / "pi"
            config.mkdir()
            (config / "models.json").write_text(json.dumps({"providers": {"local-smoke": {
                "baseUrl": f"http://127.0.0.1:{server.server_port}/v1", "api": "openai-completions",
                "apiKey": "local-fixture", "models": [{"id": "fixture", "reasoning": False,
                                                        "contextWindow": 128000, "maxTokens": 4096}]
            }}}))
            (config / "settings.json").write_text(json.dumps({"retry": {"enabled": False},
                                                              "enableInstallTelemetry": False}))
            env = dict(os.environ, PI_CODING_AGENT_DIR=str(config), PI_OFFLINE="1",
                       WAAP_PI_BIN=args.pi, XDG_RUNTIME_DIR=str(runtime),
                       GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
                       GIT_CONFIG_COUNT="2", GIT_CONFIG_KEY_0="commit.gpgsign", GIT_CONFIG_VALUE_0="false",
                       GIT_CONFIG_KEY_1="core.hooksPath", GIT_CONFIG_VALUE_1="/dev/null",
                       GIT_AUTHOR_NAME="Smoke", GIT_AUTHOR_EMAIL="smoke@example.com",
                       GIT_COMMITTER_NAME="Smoke", GIT_COMMITTER_EMAIL="smoke@example.com")
            for name in ["WAAP_PI_MODEL", "WAAP_PI_REASONING_EFFORT", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"]:
                env.pop(name, None)
            run(["git", "init", "-q", "--initial-branch=main"], cwd=repo, env=env)
            run(["git", "commit", "--allow-empty", "-qm", "seed"], cwd=repo, env=env)
            base = [waap, "--waap-root", str(repo / ".state")]
            run(base + ["init"], cwd=repo, env=env)
            for mode in ["complete", "stop"]:
                ready.clear()
                release.clear()
                requests.clear()
                agent = json.loads(run(base + ["--output-format", "json", "agent", "new", "--name", mode],
                                       cwd=repo, env=env, text="Complete the local smoke fixture."))["agent_id"]
                owner = subprocess.Popen(base + ["agent", "run", "--agent-id", agent, "--model", "local-smoke/fixture"],
                                         cwd=repo, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                try:
                    assert ready.wait(15), "real Pi did not contact the local provider"
                    if mode == "complete":
                        steering = "Include snow 雪 in your review"
                        run(base + ["agent", "steer", "--agent-id", agent, "--prompt", steering], cwd=repo, env=env)
                        release.set()
                    else:
                        run(base + ["agent", "stop", "--agent-id", agent], cwd=repo, env=env)
                    out, err = owner.communicate(timeout=15)
                    assert owner.returncode == 0, (out, err)
                    status = "completed" if mode == "complete" else "aborted"
                    record = (repo / ".state/agents" / agent / "agent.md").read_text()
                    assert f'status = "{status}"' in record, record
                    assert 'system = "pi"' in record and "session_id =" in record
                    assert not (repo / "worktrees" / agent).exists()
                    assert not list(runtime.rglob("*.sock"))
                    sessions = list((config / "sessions").rglob("*.jsonl"))
                    assert sessions, "Pi did not persist the real session"
                    if mode == "complete":
                        assert "real Pi tool execution" in run(["git", "show", f"{agent}:smoke.txt"], cwd=repo, env=env)
                        assert any(steering in json.dumps(request, ensure_ascii=False) for request in requests[1:]), requests
                    print(f"real Pi {mode}: {status}, session persisted, socket/worktree cleaned")
                finally:
                    release.set()
                    if owner.poll() is None:
                        owner.kill()
                        owner.communicate(timeout=5)
    finally:
        release.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
