mod agent;
mod app;
mod check;
mod cli;
mod frontmatter;
mod ids;
mod opencode;
mod record;
mod ticket;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
