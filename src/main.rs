mod agent;
mod app;
mod check;
mod claude;
mod cli;
mod codex;
mod frontmatter;
mod git;
mod ids;
mod opencode;
mod process;
mod record;
mod ticket;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
