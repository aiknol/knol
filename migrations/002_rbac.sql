-- RBAC: Tenant API Keys with role-based access control
-- Each tenant can have multiple API keys, each with a specific role.

-- ── tenant_api_keys table ──
CREATE TABLE IF NOT EXISTS tenant_api_keys (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  name            TEXT NOT NULL DEFAULT 'default',
  key_hash        TEXT NOT NULL UNIQUE,
  role            TEXT NOT NULL DEFAULT 'admin'
                  CHECK (role IN ('admin', 'developer', 'read_only')),
  last_used_at    TIMESTAMPTZ,
  expires_at      TIMESTAMPTZ,
  active          BOOLEAN NOT NULL DEFAULT true,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON tenant_api_keys(key_hash) WHERE active = true;
CREATE INDEX IF NOT EXISTS idx_api_keys_tenant ON tenant_api_keys(tenant_id);

-- RLS: tenant_api_keys
ALTER TABLE tenant_api_keys ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_api_keys ON tenant_api_keys;
CREATE POLICY tenant_isolation_api_keys ON tenant_api_keys
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- ── Migrate existing tenants.api_key_hash into tenant_api_keys ──
-- Every existing tenant gets an 'admin' key from their current api_key_hash.
INSERT INTO tenant_api_keys (tenant_id, name, key_hash, role)
SELECT id, 'primary', api_key_hash, 'admin'
FROM tenants
WHERE api_key_hash IS NOT NULL
  AND api_key_hash != ''
ON CONFLICT (key_hash) DO NOTHING;
