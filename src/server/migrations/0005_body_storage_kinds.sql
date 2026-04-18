alter table document_body_snapshots
  add column if not exists state_kind text;

update document_body_snapshots
set state_kind = 'full_text_merge_v1'
where state_kind is null;

alter table document_body_snapshots
  alter column state_kind set not null;

alter table document_body_revisions
  add column if not exists history_kind text;

update document_body_revisions
set history_kind = 'full_text_revision_v1'
where history_kind is null;

alter table document_body_revisions
  alter column history_kind set not null;
