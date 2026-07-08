mod agent;
mod app;
mod check;
mod cli;
mod frontmatter;
mod git;
mod ids;
mod init;
mod process;
mod record;
mod root;
#[cfg(test)]
mod test_git;
mod ticket;
mod toml;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run()
}
