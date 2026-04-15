alter table document_body_revisions
  add column if not exists workspace_cursor bigint;

update document_body_revisions
set workspace_cursor = seq
where workspace_cursor is null;

alter table document_body_revisions
  alter column workspace_cursor set not null;

create index if not exists document_body_revisions_workspace_cursor_idx
  on document_body_revisions (workspace_id, workspace_cursor, document_id, seq);

alter table document_path_history
  add column if not exists workspace_cursor bigint;

update document_path_history
set workspace_cursor = seq
where workspace_cursor is null;

alter table document_path_history
  alter column workspace_cursor set not null;

create index if not exists document_path_history_workspace_cursor_idx
  on document_path_history (workspace_id, workspace_cursor, document_id, seq);
