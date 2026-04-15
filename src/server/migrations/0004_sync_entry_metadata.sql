alter table workspaces
  add column if not exists source_repo_name text;

alter table workspaces
  add column if not exists entry_kind text;
