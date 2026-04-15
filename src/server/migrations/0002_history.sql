create table if not exists document_body_revisions (
  seq bigint generated always as identity primary key,
  workspace_id text not null references workspaces(id) on delete cascade,
  document_id text not null references documents(id) on delete cascade,
  actor_id text not null,
  base_text text not null default '',
  body_text text not null default '',
  conflicted boolean not null default false,
  created_at timestamptz not null default now()
);

create index if not exists document_body_revisions_workspace_document_seq_idx
  on document_body_revisions (workspace_id, document_id, seq);

create table if not exists document_path_history (
  seq bigint generated always as identity primary key,
  workspace_id text not null references workspaces(id) on delete cascade,
  document_id text not null references documents(id) on delete cascade,
  actor_id text not null,
  mount_path text not null,
  relative_path text not null,
  deleted boolean not null default false,
  event_kind text not null,
  created_at timestamptz not null default now()
);

create index if not exists document_path_history_workspace_document_seq_idx
  on document_path_history (workspace_id, document_id, seq);
