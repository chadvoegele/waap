# Work log: tt-warn-when-a-nested-waap-project-exists-below-the-repo-root

- Read ticket and src/cli.rs, src/app.rs, src/record.rs.
- Plan: add `find_nested_waap_projects(repo_root) -> Vec<PathBuf>` to src/record.rs.
  Walk repo_root, skip .git/worktrees/target/node_modules, skip repo_root/.waap itself,
  stop descending once a .waap dir is found (report parent dir), return relative paths.
- Call from app::run, print warning to stderr if non-empty, before dispatching command.
- Marked ticket in-progress.
