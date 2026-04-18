alter table document_body_revisions
  add column if not exists checkpoint_anchor_seq bigint;

update document_body_revisions
set checkpoint_anchor_seq = seq
where checkpoint_anchor_seq is null
  and history_kind <> 'yrs_text_update_v1';
