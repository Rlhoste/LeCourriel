-- Le courriel — Supabase schema.
-- Run once in your project: Supabase dashboard → SQL Editor → paste → Run.
-- Safe to run again (idempotent).
--
-- Every row belongs to a user (user_id = auth.uid()). Row-level security makes
-- PostgreSQL itself refuse access to other users' rows, whatever client is used.
-- Mail passwords and API keys are never stored here: they stay on each computer.
--
-- The tables are plain relational tables (not JSON blobs) so a future company
-- reporting / steering system can query them. Give such a system its own role
-- and policies; never weaken the per-user policies below.

-- ── Tables ──────────────────────────────────────────────────────────────────

-- One row per user: display preferences and the Claude model (no API key).
create table if not exists public.user_prefs (
  user_id      uuid primary key default auth.uid() references auth.users (id) on delete cascade,
  display_name text not null default '',
  font_scale   real not null default 1.0,
  theme        text not null default 'brutalist',
  language     text not null default 'en',
  claude_model text not null default '',   -- the AI model (name kept for compatibility)
  updated_at   timestamptz not null default now()
);
-- Added with the choice of AI provider (anthropic, openai, mistral, ollama, custom).
alter table public.user_prefs add column if not exists ai_provider text not null default 'anthropic';

-- Projects the AI sorts emails into.
create table if not exists public.projects (
  user_id     uuid not null default auth.uid() references auth.users (id) on delete cascade,
  name        text not null,
  description text not null default '',
  urgency     text not null default 'normal' check (urgency in ('normal', 'high')),
  primary key (user_id, name)
);

-- The task board, per mailbox ("user@imap-host").
create table if not exists public.tasks (
  user_id        uuid not null default auth.uid() references auth.users (id) on delete cascade,
  mailbox        text not null,
  local_id       bigint not null,
  title          text not null,
  status         text not null check (status in ('todo', 'in_progress', 'done', 'given_up')),
  project        text,
  source_uid     bigint,          -- IMAP UID of the email the task came from
  source_from    text not null default '',
  source_subject text not null default '',
  ai_verdict     text,            -- 'done' | 'obsolete' when the AI moved the task
  primary key (user_id, mailbox, local_id)
);

-- AI classification of each email (no email bodies).
create table if not exists public.email_insights (
  user_id      uuid not null default auth.uid() references auth.users (id) on delete cascade,
  mailbox      text not null,
  message_id   text not null,
  priority     text not null,     -- Urgent | HighPriority | OtherPriority | Information | Threat | Task
  summary      text not null default '',
  project      text,
  deadline     date,
  task         text,
  meeting_link text,
  updated_at   timestamptz not null default now(),
  primary key (user_id, mailbox, message_id)
);

-- Small per-mailbox documents kept as JSON: swipe dismissals, manual project tags.
create table if not exists public.app_documents (
  user_id    uuid not null default auth.uid() references auth.users (id) on delete cascade,
  mailbox    text not null,
  name       text not null check (name in ('dismissed', 'overrides')),
  data       jsonb not null,
  updated_at timestamptz not null,
  primary key (user_id, mailbox, name)
);

-- When each synced document last changed (last-write-wins), plus task-board metadata.
create table if not exists public.sync_state (
  user_id    uuid not null default auth.uid() references auth.users (id) on delete cascade,
  mailbox    text not null,       -- '' for per-user documents (prefs)
  doc        text not null,
  updated_at timestamptz not null,
  meta       jsonb not null default '{}'::jsonb,
  primary key (user_id, mailbox, doc)
);

-- ── Row-level security: each user sees and changes only their own rows ──────

do $$
declare t text;
begin
  foreach t in array array['user_prefs', 'projects', 'tasks', 'email_insights', 'app_documents', 'sync_state'] loop
    execute format('alter table public.%I enable row level security', t);
    execute format('alter table public.%I force row level security', t);
    execute format('drop policy if exists "own rows" on public.%I', t);
    execute format(
      'create policy "own rows" on public.%I for all to authenticated
         using (user_id = auth.uid()) with check (user_id = auth.uid())', t);
    execute format('revoke all on public.%I from anon', t);
    execute format('grant select, insert, update, delete on public.%I to authenticated', t);
  end loop;
end $$;

-- ── Sync functions (run with the caller's rights, so RLS applies) ───────────

create or replace function public.courriel_require_user() returns uuid
language plpgsql stable security invoker set search_path = public as $$
begin
  if auth.uid() is null then
    raise exception 'not signed in' using errcode = '42501';
  end if;
  return auth.uid();
end $$;

-- Store a document if it is newer than what is stored (last write wins).
create or replace function public.courriel_push(p_mailbox text, p_doc text, p_data jsonb, p_updated_at timestamptz)
returns boolean
language plpgsql security invoker set search_path = public as $$
declare
  uid uuid := courriel_require_user();
  box text := case when p_doc = 'prefs' then '' else p_mailbox end;
  stored timestamptz;
begin
  select updated_at into stored from sync_state where user_id = uid and mailbox = box and doc = p_doc;
  if stored is not null and stored > p_updated_at then
    return false;   -- the database already has a newer version
  end if;

  if p_doc = 'tasks' then
    delete from tasks where user_id = uid and mailbox = box;
    insert into tasks (user_id, mailbox, local_id, title, status, project, source_uid, source_from, source_subject, ai_verdict)
    select uid, box, t.id, t.title, t.status, t.project, t.source_uid,
           coalesce(t.source_from, ''), coalesce(t.source_subject, ''), t.ai_verdict
    from jsonb_to_recordset(coalesce(p_data -> 'tasks', '[]'::jsonb)) as t(
      id bigint, title text, status text, project text, source_uid bigint,
      source_from text, source_subject text, ai_verdict text);
    insert into sync_state (user_id, mailbox, doc, updated_at, meta)
    values (uid, box, p_doc, p_updated_at, jsonb_build_object(
      'next_id', coalesce(p_data -> 'next_id', '0'::jsonb),
      'deleted_sources', coalesce(p_data -> 'deleted_sources', '[]'::jsonb)))
    on conflict (user_id, mailbox, doc) do update set updated_at = excluded.updated_at, meta = excluded.meta;

  elsif p_doc = 'prefs' then
    insert into user_prefs (user_id, display_name, font_scale, theme, language, claude_model, ai_provider, updated_at)
    values (uid,
            coalesce(p_data #>> '{user,display_name}', ''),
            coalesce((p_data #>> '{user,font_scale}')::real, 1.0),
            coalesce(p_data #>> '{user,theme}', 'brutalist'),
            coalesce(p_data #>> '{user,language}', 'en'),
            coalesce(p_data ->> 'ai_model', p_data ->> 'claude_model', ''),
            coalesce(p_data ->> 'ai_provider', 'anthropic'),
            p_updated_at)
    on conflict (user_id) do update set
      display_name = excluded.display_name, font_scale = excluded.font_scale, theme = excluded.theme,
      language = excluded.language, claude_model = excluded.claude_model, ai_provider = excluded.ai_provider,
      updated_at = excluded.updated_at;
    delete from projects where user_id = uid;
    insert into projects (user_id, name, description, urgency)
    select uid, p.name, coalesce(p.description, ''), coalesce(p.urgency, 'normal')
    from jsonb_to_recordset(coalesce(p_data -> 'projects', '[]'::jsonb)) as p(name text, description text, urgency text);
    insert into sync_state (user_id, mailbox, doc, updated_at)
    values (uid, box, p_doc, p_updated_at)
    on conflict (user_id, mailbox, doc) do update set updated_at = excluded.updated_at;

  elsif p_doc in ('dismissed', 'overrides') then
    insert into app_documents (user_id, mailbox, name, data, updated_at)
    values (uid, box, p_doc, p_data, p_updated_at)
    on conflict (user_id, mailbox, name) do update set data = excluded.data, updated_at = excluded.updated_at;
    insert into sync_state (user_id, mailbox, doc, updated_at)
    values (uid, box, p_doc, p_updated_at)
    on conflict (user_id, mailbox, doc) do update set updated_at = excluded.updated_at;

  else
    raise exception 'unknown document %', p_doc;
  end if;
  return true;
end $$;

-- Read a document back in the app's local file format: {"data": …, "updated_at": …} or null.
create or replace function public.courriel_pull(p_mailbox text, p_doc text)
returns jsonb
language plpgsql stable security invoker set search_path = public as $$
declare
  uid uuid := courriel_require_user();
  box text := case when p_doc = 'prefs' then '' else p_mailbox end;
  st sync_state;
  result jsonb;
begin
  select * into st from sync_state where user_id = uid and mailbox = box and doc = p_doc;
  if not found then
    return null;
  end if;

  if p_doc = 'tasks' then
    select jsonb_build_object(
      'next_id', coalesce(st.meta -> 'next_id', '0'::jsonb),
      'deleted_sources', coalesce(st.meta -> 'deleted_sources', '[]'::jsonb),
      'tasks', coalesce(jsonb_agg(jsonb_build_object(
        'id', t.local_id, 'title', t.title, 'status', t.status, 'project', t.project,
        'source_uid', t.source_uid, 'source_from', t.source_from,
        'source_subject', t.source_subject, 'ai_verdict', t.ai_verdict) order by t.local_id), '[]'::jsonb))
    into result
    from tasks t where t.user_id = uid and t.mailbox = box;

  elsif p_doc = 'prefs' then
    select jsonb_build_object(
      'user', jsonb_build_object('display_name', u.display_name, 'font_scale', u.font_scale,
                                 'theme', u.theme, 'language', u.language),
      'ai_provider', u.ai_provider,
      'ai_model', u.claude_model,
      'projects', coalesce((select jsonb_agg(jsonb_build_object('name', p.name, 'description', p.description,
                                                                'urgency', p.urgency) order by p.name)
                            from projects p where p.user_id = uid), '[]'::jsonb))
    into result
    from user_prefs u where u.user_id = uid;

  else
    select d.data into result from app_documents d
    where d.user_id = uid and d.mailbox = box and d.name = p_doc;
  end if;

  if result is null then
    return null;
  end if;
  return jsonb_build_object('data', result, 'updated_at', st.updated_at);
end $$;

-- AI results for these Message-IDs: {"<id>": {priority, summary, project, deadline, task, meeting_link}}.
create or replace function public.courriel_lookup_insights(p_mailbox text, p_ids text[])
returns jsonb
language sql stable security invoker set search_path = public as $$
  select coalesce(jsonb_object_agg(e.message_id, jsonb_build_object(
           'priority', e.priority, 'summary', e.summary, 'project', e.project,
           'deadline', e.deadline, 'task', e.task, 'meeting_link', e.meeting_link)), '{}'::jsonb)
  from email_insights e
  where e.user_id = courriel_require_user() and e.mailbox = p_mailbox and e.message_id = any (p_ids);
$$;

-- Save AI results: p_items = {"<id>": {priority, summary, …}}.
create or replace function public.courriel_store_insights(p_mailbox text, p_items jsonb)
returns integer
language plpgsql security invoker set search_path = public as $$
declare
  uid uuid := courriel_require_user();
  n integer;
begin
  insert into email_insights (user_id, mailbox, message_id, priority, summary, project, deadline, task, meeting_link, updated_at)
  select uid, p_mailbox, i.key, i.value ->> 'priority', coalesce(i.value ->> 'summary', ''),
         i.value ->> 'project', (i.value ->> 'deadline')::date, i.value ->> 'task', i.value ->> 'meeting_link', now()
  from jsonb_each(p_items) as i
  on conflict (user_id, mailbox, message_id) do update set
    priority = excluded.priority, summary = excluded.summary, project = excluded.project,
    deadline = excluded.deadline, task = excluded.task, meeting_link = excluded.meeting_link,
    updated_at = now();
  get diagnostics n = row_count;
  return n;
end $$;

-- Only signed-in users may call the sync functions.
do $$
declare f text;
begin
  foreach f in array array[
    'courriel_require_user()',
    'courriel_push(text, text, jsonb, timestamptz)',
    'courriel_pull(text, text)',
    'courriel_lookup_insights(text, text[])',
    'courriel_store_insights(text, jsonb)'] loop
    execute format('revoke execute on function public.%s from public, anon', f);
    execute format('grant execute on function public.%s to authenticated', f);
  end loop;
end $$;
