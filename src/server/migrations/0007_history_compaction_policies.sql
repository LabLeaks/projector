create table if not exists history_compaction_policies (
  workspace_id text not null references workspaces(id) on delete cascade,
  repo_relative_path text not null,
  revisions integer not null,
  frequency integer not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (workspace_id, repo_relative_path)
);
