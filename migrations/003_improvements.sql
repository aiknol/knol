-- Migration 003: Infrastructure improvements
-- Soft delete support, configurable search/graph/resilience parameters

-- ── Soft Delete Support ──
ALTER TABLE memories ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_memories_soft_delete
    ON memories (status, deleted_at)
    WHERE status = 'deleted';

-- ── Seed New Config Keys ──

-- Ensure OSS deployments have a runtime config store.
-- Enterprise migrations also define this table; keep schema compatible.
CREATE TABLE IF NOT EXISTS system_config (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key          TEXT NOT NULL UNIQUE,
    value        JSONB NOT NULL DEFAULT '{}',
    value_type   TEXT NOT NULL DEFAULT 'string'
                     CHECK (value_type IN ('string','number','boolean','json','string_array')),
    category     TEXT NOT NULL DEFAULT 'general',
    description  TEXT NOT NULL DEFAULT '',
    env_override TEXT,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_system_config_category ON system_config (category);

-- Graph traversal configs
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('graph.max_traversal_depth', '10', 'number', 'graph',
     'Maximum allowed depth for N-hop graph traversal', 'GRAPH_MAX_TRAVERSAL_DEPTH'),
    ('graph.max_traversal_results', '1000', 'number', 'graph',
     'Maximum results from a single graph traversal query', 'GRAPH_MAX_TRAVERSAL_RESULTS'),
    ('graph.max_path_depth', '10', 'number', 'graph',
     'Maximum depth for shortest-path queries', 'GRAPH_MAX_PATH_DEPTH')
ON CONFLICT (key) DO NOTHING;

-- Search / BM25 configs
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('search.rrf_k', '60.0', 'number', 'search',
     'Reciprocal Rank Fusion constant k (higher = more weight to lower ranks)', 'SEARCH_RRF_K'),
    ('search.bm25_weights', '[0.1, 0.2, 0.4, 1.0]', 'json', 'search',
     'ts_rank_cd weights for D, C, B, A text fields', '')
ON CONFLICT (key) DO NOTHING;

-- Per-intent fusion weights: vector
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('search.vector_weight_preference', '1.0', 'number', 'search',
     'Vector search weight for preference intent queries', ''),
    ('search.vector_weight_temporal', '0.5', 'number', 'search',
     'Vector search weight for temporal intent queries', ''),
    ('search.vector_weight_relational', '0.4', 'number', 'search',
     'Vector search weight for relational intent queries', ''),
    ('search.vector_weight_general', '0.7', 'number', 'search',
     'Vector search weight for general intent queries', '')
ON CONFLICT (key) DO NOTHING;

-- Per-intent fusion weights: bm25
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('search.bm25_weight_preference', '0.3', 'number', 'search',
     'BM25 search weight for preference intent queries', ''),
    ('search.bm25_weight_temporal', '0.8', 'number', 'search',
     'BM25 search weight for temporal intent queries', ''),
    ('search.bm25_weight_relational', '0.2', 'number', 'search',
     'BM25 search weight for relational intent queries', ''),
    ('search.bm25_weight_general', '0.6', 'number', 'search',
     'BM25 search weight for general intent queries', '')
ON CONFLICT (key) DO NOTHING;

-- Per-intent fusion weights: graph
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('search.graph_weight_preference', '0.2', 'number', 'search',
     'Graph weight for preference intent queries', ''),
    ('search.graph_weight_temporal', '0.7', 'number', 'search',
     'Graph weight for temporal intent queries', ''),
    ('search.graph_weight_relational', '1.0', 'number', 'search',
     'Graph weight for relational intent queries', ''),
    ('search.graph_weight_general', '0.5', 'number', 'search',
     'Graph weight for general intent queries', '')
ON CONFLICT (key) DO NOTHING;

-- Per-intent fusion weights: scope cascade
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('search.scope_weight_preference', '0.4', 'number', 'search',
     'Scope cascade weight for preference intent queries', ''),
    ('search.scope_weight_temporal', '0.3', 'number', 'search',
     'Scope cascade weight for temporal intent queries', ''),
    ('search.scope_weight_relational', '0.3', 'number', 'search',
     'Scope cascade weight for relational intent queries', ''),
    ('search.scope_weight_general', '0.4', 'number', 'search',
     'Scope cascade weight for general intent queries', '')
ON CONFLICT (key) DO NOTHING;

-- Resilience configs
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('resilience.redis_required', 'false', 'boolean', 'resilience',
     'If false, services degrade gracefully when Redis is unavailable', 'RESILIENCE_REDIS_REQUIRED'),
    ('resilience.skip_rate_limit_on_redis_failure', 'false', 'boolean', 'resilience',
     'Skip rate limiting when Redis is down (vs rejecting all requests)', '')
ON CONFLICT (key) DO NOTHING;

-- Soft delete / retention configs
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('retention.soft_delete_days', '30', 'number', 'retention',
     'Days to retain soft-deleted memories before permanent removal', 'RETENTION_SOFT_DELETE_DAYS'),
    ('retention.soft_delete_default', 'true', 'boolean', 'retention',
     'Use soft delete by default (vs hard delete)', 'RETENTION_SOFT_DELETE_DEFAULT')
ON CONFLICT (key) DO NOTHING;

-- Gateway rate limit tiers (configurable per plan)
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('gateway.rate_limit_free', '10', 'number', 'gateway',
     'Max requests per minute for free plan', ''),
    ('gateway.rate_limit_developer', '100', 'number', 'gateway',
     'Max requests per minute for developer plan', ''),
    ('gateway.rate_limit_pro', '500', 'number', 'gateway',
     'Max requests per minute for pro plan', ''),
    ('gateway.rate_limit_team', '2000', 'number', 'gateway',
     'Max requests per minute for team plan', ''),
    ('gateway.rate_limit_enterprise', '10000', 'number', 'gateway',
     'Max requests per minute for enterprise plan', '')
ON CONFLICT (key) DO NOTHING;

-- Gateway guardrails
INSERT INTO system_config (key, value, value_type, category, description, env_override) VALUES
    ('gateway.max_body_size_bytes', '10485760', 'number', 'gateway',
     'Maximum request body size in bytes (default 10MB)', 'GATEWAY_MAX_BODY_SIZE'),
    ('gateway.max_webhooks_per_tenant', '50', 'number', 'gateway',
     'Maximum webhooks per tenant', 'GATEWAY_MAX_WEBHOOKS_PER_TENANT')
ON CONFLICT (key) DO NOTHING;
