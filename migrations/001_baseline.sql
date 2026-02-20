-- Consolidated OSS baseline migration
-- Generated during development to reduce migration fan-out.

-- >>> BEGIN 001_tenants.sql
-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "vector";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";

-- Tenants table
CREATE TABLE IF NOT EXISTS tenants (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name            TEXT NOT NULL,
  slug            TEXT NOT NULL UNIQUE,
  plan            TEXT NOT NULL DEFAULT 'free'
                  CHECK (plan IN ('free','developer','pro','team','enterprise')),
  config          JSONB NOT NULL DEFAULT '{
    "extraction_model": "claude-haiku-4-5-20251001",
    "embedding_model": "voyage-3-lite",
    "embedding_dim": 1024,
    "decay_lambda": 0.01,
    "importance_threshold": 0.1,
    "max_memories_per_user": 100000,
    "retention_days": null,
    "pii_redaction": false,
    "custom_ontology": null
  }'::jsonb,
  api_key_hash    TEXT NOT NULL,
  usage_ops_month INTEGER NOT NULL DEFAULT 0,
  usage_limit     INTEGER,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- RLS on tenants
ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_self_access ON tenants;
CREATE POLICY tenant_self_access ON tenants
  USING (id = current_setting('app.tenant_id', true)::uuid);

-- <<< END 001_tenants.sql

-- >>> BEGIN 002_episodes.sql
CREATE TABLE IF NOT EXISTS episodes (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id),
  user_id         UUID,
  session_id      TEXT,
  agent_id        TEXT,
  content         TEXT NOT NULL,
  role            TEXT NOT NULL CHECK (role IN ('user','assistant','system','tool')),
  event_time      TIMESTAMPTZ NOT NULL DEFAULT now(),
  ingested_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  storage_key     TEXT,
  content_hash    TEXT NOT NULL,
  metadata        JSONB DEFAULT '{}',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_episodes_tenant_user ON episodes(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(tenant_id, session_id);
CREATE INDEX IF NOT EXISTS idx_episodes_time ON episodes(tenant_id, event_time DESC);
CREATE INDEX IF NOT EXISTS idx_episodes_hash ON episodes(tenant_id, content_hash);

ALTER TABLE episodes ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_episodes ON episodes;
CREATE POLICY tenant_isolation_episodes ON episodes
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- <<< END 002_episodes.sql

-- >>> BEGIN 003_memories.sql
CREATE TABLE IF NOT EXISTS memories (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id),
  user_id         UUID,
  scope           TEXT NOT NULL DEFAULT 'user'
                  CHECK (scope IN ('user','team','project','agent','org')),
  kind            TEXT NOT NULL
                  CHECK (kind IN ('preference','fact','task','event','relationship','summary','procedure')),
  content         TEXT NOT NULL,
  content_json    JSONB,
  confidence      REAL NOT NULL DEFAULT 0.8 CHECK (confidence >= 0 AND confidence <= 1),
  importance      REAL NOT NULL DEFAULT 0.5 CHECK (importance >= 0 AND importance <= 1),
  status          TEXT NOT NULL DEFAULT 'active'
                  CHECK (status IN ('active','superseded','archived','deleted')),
  valid_from      TIMESTAMPTZ NOT NULL DEFAULT now(),
  valid_to        TIMESTAMPTZ,
  event_time      TIMESTAMPTZ,
  ingested_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  source_episode_id UUID REFERENCES episodes(id),
  created_by      TEXT NOT NULL DEFAULT 'system'
                  CHECK (created_by IN ('system','user','admin','connector')),
  tags            TEXT[] DEFAULT '{}',
  metadata        JSONB DEFAULT '{}',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_memories_tenant_user ON memories(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(tenant_id, scope);
CREATE INDEX IF NOT EXISTS idx_memories_kind ON memories(tenant_id, kind);
CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(tenant_id, status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_memories_valid ON memories(tenant_id, valid_from, valid_to);
CREATE INDEX IF NOT EXISTS idx_memories_tags ON memories USING GIN(tags);
CREATE INDEX IF NOT EXISTS idx_memories_metadata ON memories USING GIN(metadata);
-- Full-text search index for BM25
CREATE INDEX IF NOT EXISTS idx_memories_fts ON memories USING GIN(to_tsvector('english', content));

ALTER TABLE memories ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_memories ON memories;
CREATE POLICY tenant_isolation_memories ON memories
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Memory vectors table (pgvector)
CREATE TABLE IF NOT EXISTS memory_vectors (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  memory_id       UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  tenant_id       UUID NOT NULL,
  user_id         UUID,
  scope           TEXT NOT NULL,
  kind            TEXT NOT NULL,
  status          TEXT NOT NULL DEFAULT 'active',
  valid_from      TIMESTAMPTZ NOT NULL,
  valid_to        TIMESTAMPTZ,
  embedding       vector(1024) NOT NULL,
  content_hash    TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_vectors_embedding ON memory_vectors
  USING hnsw (embedding vector_cosine_ops) WITH (m = 16, ef_construction = 200);
CREATE INDEX IF NOT EXISTS idx_vectors_tenant ON memory_vectors(tenant_id, status);
CREATE INDEX IF NOT EXISTS idx_vectors_dedup ON memory_vectors(tenant_id, content_hash);

ALTER TABLE memory_vectors ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_vectors ON memory_vectors;
CREATE POLICY tenant_isolation_vectors ON memory_vectors
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- <<< END 003_memories.sql

-- >>> BEGIN 004_entities_edges.sql
CREATE TABLE IF NOT EXISTS entities (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id),
  name            TEXT NOT NULL,
  entity_type     TEXT NOT NULL,
  summary         TEXT,
  attributes      JSONB DEFAULT '{}',
  embedding       vector(1024),
  valid_from      TIMESTAMPTZ NOT NULL DEFAULT now(),
  valid_to        TIMESTAMPTZ,
  status          TEXT NOT NULL DEFAULT 'active'
                  CHECK (status IN ('active','merged','deleted')),
  merged_into     UUID REFERENCES entities(id),
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_entities_tenant ON entities(tenant_id);
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(tenant_id, entity_type);
CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(tenant_id, name);
CREATE INDEX IF NOT EXISTS idx_entities_embedding ON entities USING hnsw (embedding vector_cosine_ops);
CREATE UNIQUE INDEX IF NOT EXISTS idx_entities_dedup ON entities(tenant_id, name, entity_type) WHERE status = 'active';
-- Full-text search on entity names
CREATE INDEX IF NOT EXISTS idx_entities_fts ON entities USING GIN(to_tsvector('english', name || ' ' || COALESCE(summary, '')));

ALTER TABLE entities ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_entities ON entities;
CREATE POLICY tenant_isolation_entities ON entities
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

CREATE TABLE IF NOT EXISTS edges (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id),
  source_entity_id UUID NOT NULL REFERENCES entities(id),
  target_entity_id UUID NOT NULL REFERENCES entities(id),
  rel_type        TEXT NOT NULL,
  properties      JSONB DEFAULT '{}',
  weight          REAL NOT NULL DEFAULT 1.0,
  source_episode_id UUID REFERENCES episodes(id),
  valid_from      TIMESTAMPTZ NOT NULL DEFAULT now(),
  valid_to        TIMESTAMPTZ,
  status          TEXT NOT NULL DEFAULT 'active',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(tenant_id, source_entity_id) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(tenant_id, target_entity_id) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_edges_rel ON edges(tenant_id, rel_type);
CREATE INDEX IF NOT EXISTS idx_edges_temporal ON edges(tenant_id, valid_from, valid_to);

ALTER TABLE edges ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_edges ON edges;
CREATE POLICY tenant_isolation_edges ON edges
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- <<< END 004_entities_edges.sql

-- >>> BEGIN 005_working_procedural.sql
CREATE TABLE IF NOT EXISTS working_memory (
  session_id      TEXT NOT NULL,
  tenant_id       UUID NOT NULL REFERENCES tenants(id),
  user_id         UUID,
  agent_id        TEXT,
  summary         TEXT,
  active_facts    UUID[] DEFAULT '{}',
  active_procs    UUID[] DEFAULT '{}',
  scratchpad      JSONB DEFAULT '{}',
  turn_count      INTEGER NOT NULL DEFAULT 0,
  last_updated    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (tenant_id, session_id)
);

ALTER TABLE working_memory ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_working ON working_memory;
CREATE POLICY tenant_isolation_working ON working_memory
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

CREATE TABLE IF NOT EXISTS procedural_memories (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id),
  scope           TEXT NOT NULL DEFAULT 'user',
  scope_id        TEXT,
  user_id         UUID,
  agent_id        TEXT,
  description     TEXT NOT NULL,
  trigger_condition TEXT,
  procedure_steps TEXT NOT NULL,
  success_count   INTEGER NOT NULL DEFAULT 0,
  fail_count      INTEGER NOT NULL DEFAULT 0,
  embedding       vector(1024),
  status          TEXT NOT NULL DEFAULT 'active',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_used       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_procs_tenant ON procedural_memories(tenant_id, status);
CREATE INDEX IF NOT EXISTS idx_procs_embedding ON procedural_memories USING hnsw (embedding vector_cosine_ops);

ALTER TABLE procedural_memories ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_procs ON procedural_memories;
CREATE POLICY tenant_isolation_procs ON procedural_memories
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- <<< END 005_working_procedural.sql
