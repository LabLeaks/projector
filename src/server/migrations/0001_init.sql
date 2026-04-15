create table if not exists workspaces (
  id text primary key,
  owner_actor_id text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists workspace_mounts (
  workspace_id text not null references workspaces(id) on delete cascade,
  mount_path text not null,
  created_at timestamptz not null default now(),
  primary key (workspace_id, mount_path)
);

create table if not exists documents (
  id text primary key,
  workspace_id text not null references workspaces(id) on delete cascade,
  kind text not null,
  created_at timestamptz not null default now()
);

create table if not exists document_paths (
  document_id text primary key references documents(id) on delete cascade,
  workspace_id text not null references workspaces(id) on delete cascade,
  mount_path text not null,
  relative_path text not null,
  deleted boolean not null default false,
  manifest_version bigint not null default 0,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create unique index if not exists document_paths_live_path_idx
  on document_paths (workspace_id, mount_path, relative_path)
  where deleted = false;

create table if not exists document_body_snapshots (
  document_id text primary key references documents(id) on delete cascade,
  workspace_id text not null references workspaces(id) on delete cascade,
  body_text text not null default '',
  yjs_state bytea,
  state_vector bytea,
  compacted_through_seq bigint not null default 0,
  updated_at timestamptz not null default now()
);

create table if not exists document_body_updates (
  document_id text not null references documents(id) on delete cascade,
  workspace_id text not null references workspaces(id) on delete cascade,
  seq bigint generated always as identity,
  actor_id text not null,
  update_blob bytea not null,
  created_at timestamptz not null default now(),
  primary key (document_id, seq)
);

create table if not exists provenance_events (
  seq bigint generated always as identity primary key,
  workspace_id text not null references workspaces(id) on delete cascade,
  actor_id text not null,
  document_id text references documents(id) on delete set null,
  mount_path text,
  relative_path text,
  event_kind text not null,
  summary text not null,
  created_at timestamptz not null default now()
);
