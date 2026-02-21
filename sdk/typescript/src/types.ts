/**
 * Knol Memory Platform SDK - Type Definitions
 * Comprehensive TypeScript types for all API operations
 */

// ============================================================================
// Memory Operations Types
// ============================================================================

export interface MemoryMetadata {
  [key: string]: unknown;
}

export interface TemporalFilter {
  start_date?: string;
  end_date?: string;
  recency_days?: number;
  /** Simulate memory state at a specific point in time. ISO 8601 string. */
  point_in_time?: string;
}

export type MemoryScope = 'private' | 'team' | 'organization' | 'public';
export type MemoryKind = 'fact' | 'insight' | 'interaction' | 'context' | 'summary';
export type MemoryStatus = 'active' | 'archived' | 'deleted';

export interface Memory {
  id: string;
  content: string;
  user_id: string;
  role?: string;
  session_id?: string;
  agent_id?: string;
  scope?: MemoryScope;
  kind?: MemoryKind;
  status?: MemoryStatus;
  importance?: number;
  confidence?: number;
  tags?: string[];
  metadata?: MemoryMetadata;
  created_at?: string;
  updated_at?: string;
  expires_at?: string;
}

export interface WriteMemoryRequest {
  content: string;
  user_id: string;
  role?: string;
  session_id?: string;
  agent_id?: string;
  scope?: MemoryScope;
  kind?: MemoryKind;
  importance?: number;
  tags?: string[];
  metadata?: MemoryMetadata;
  expires_at?: string;
}

export interface UpdateMemoryRequest {
  content?: string;
  status?: MemoryStatus;
  importance?: number;
  scope?: MemoryScope;
  tags?: string[];
  metadata?: MemoryMetadata;
}

export interface SearchMemoryRequest {
  query: string;
  user_id?: string;
  scope?: MemoryScope | MemoryScope[];
  kind?: MemoryKind | MemoryKind[];
  limit?: number;
  min_confidence?: number;
  temporal_filter?: TemporalFilter;
  session_id?: string;
  agent_id?: string;
  tags?: string[];
  entity_types?: string[];
  min_importance?: number;
  apply_decay?: boolean;
  graph_depth?: number;
}

export interface SearchMemoryResponse {
  results: MemoryWithScore[];
  total: number;
  has_more: boolean;
}

export interface MemoryWithScore extends Memory {
  score: number;
  relevance_score?: number;
}

export interface BatchWriteMemoryRequest {
  memories: WriteMemoryRequest[];
  parallel?: boolean;
}

export interface BatchWriteMemoryResponse {
  created: string[];
  failed: BatchWriteError[];
  total: number;
}

export interface BatchWriteError {
  index: number;
  error: string;
}

export interface ExportMemoriesRequest {
  user_id?: string;
  scope?: MemoryScope;
  format?: 'json' | 'jsonl' | 'csv';
  include_metadata?: boolean;
}

export interface ExportMemoriesResponse {
  url: string;
  expires_at: string;
  total_records: number;
}

export interface ImportMemoriesRequest {
  data: WriteMemoryRequest[];
  update_existing?: boolean;
}

export interface ImportMemoriesResponse {
  imported: number;
  skipped: number;
  errors: BatchWriteError[];
}

// ============================================================================
// Graph Operations Types
// ============================================================================

export interface Entity {
  id: string;
  name: string;
  entity_type: string;
  description?: string;
  properties?: Record<string, unknown>;
  created_at?: string;
  updated_at?: string;
}

export interface Edge {
  id: string;
  source_id: string;
  target_id: string;
  relation_type: string;
  weight?: number;
  metadata?: Record<string, unknown>;
  created_at?: string;
}

export interface EntityListResponse {
  entities: Entity[];
  total: number;
  has_more: boolean;
}

export interface EntityEdgesResponse {
  edges: Edge[];
  total: number;
}

export interface NeighborsResponse {
  neighbors: Entity[];
  edges: Edge[];
  total: number;
}

export interface ExpandedGraph {
  center: Entity;
  first_hop: {
    entities: Entity[];
    edges: Edge[];
  };
  second_hop: {
    entities: Entity[];
    edges: Edge[];
  };
}

export interface TraversalResponse {
  root: Entity;
  paths: GraphPath[];
  visited_count: number;
}

export interface GraphPath {
  nodes: Entity[];
  edges: Edge[];
  depth: number;
}

export interface PathFinderResponse {
  path: GraphPath | null;
  distance: number;
  found: boolean;
}

// ============================================================================
// Webhook Types
// ============================================================================

export type WebhookEvent = 'memory.created' | 'memory.updated' | 'memory.deleted' | 'entity.created' | 'entity.updated';

export interface Webhook {
  id: string;
  url: string;
  events: WebhookEvent[];
  active: boolean;
  created_at?: string;
  updated_at?: string;
  secret?: string;
}

export interface CreateWebhookRequest {
  url: string;
  events: WebhookEvent[];
  active?: boolean;
  /** Optional secret for HMAC signature verification. */
  secret?: string;
  description?: string;
}

export interface PolicyResponse {
  id: string;
  name?: string;
  enabled: boolean;
  [key: string]: unknown;
}

export interface CreatePolicyRequest {
  [key: string]: unknown;
}

export interface ListWebhooksResponse {
  webhooks: Webhook[];
  total: number;
}

// ============================================================================
// Admin Types
// ============================================================================

export interface TenantUsage {
  tenant_id: string;
  memory_count: number;
  entity_count: number;
  edge_count: number;
  storage_bytes: number;
  api_calls_month: number;
  webhook_count: number;
  created_at: string;
}

export interface AuditLogEntry {
  id: string;
  timestamp: string;
  user_id: string;
  action: string;
  resource_type: string;
  resource_id: string;
  status: 'success' | 'failure';
  details?: Record<string, unknown>;
}

export interface ListAuditLogResponse {
  entries: AuditLogEntry[];
  total: number;
  has_more: boolean;
}

// ============================================================================
// Soft Delete / Restore Types
// ============================================================================

export interface DeleteMemoryOptions {
  /** If true, permanently delete (requires Admin role). Default: soft delete. */
  permanent?: boolean;
}

export interface RestoreMemoryResponse {
  id: string;
  status: 'restored';
}

// ============================================================================
// Query Builder Types
// ============================================================================

export interface SearchQueryBuilder {
  query(q: string): SearchQueryBuilder;
  userId(id: string): SearchQueryBuilder;
  scope(scope: MemoryScope | MemoryScope[]): SearchQueryBuilder;
  kind(kind: MemoryKind | MemoryKind[]): SearchQueryBuilder;
  limit(n: number): SearchQueryBuilder;
  minConfidence(score: number): SearchQueryBuilder;
  temporalFilter(filter: TemporalFilter): SearchQueryBuilder;
  sessionId(id: string): SearchQueryBuilder;
  agentId(id: string): SearchQueryBuilder;
  tags(tags: string[]): SearchQueryBuilder;
  entityTypes(types: string[]): SearchQueryBuilder;
  minImportance(score: number): SearchQueryBuilder;
  applyDecay(apply: boolean): SearchQueryBuilder;
  graphDepth(depth: number): SearchQueryBuilder;
  build(): SearchMemoryRequest;
}

// ============================================================================
// Error Types
// ============================================================================

export interface ErrorResponse {
  error: string;
  message: string;
  details?: Record<string, unknown>;
  request_id?: string;
}

export class KnolError extends Error {
  constructor(
    message: string,
    public statusCode: number,
    public requestId?: string,
    public details?: Record<string, unknown>
  ) {
    super(message);
    this.name = 'KnolError';
    Object.setPrototypeOf(this, KnolError.prototype);
  }

  static isKnolError(error: unknown): error is KnolError {
    return error instanceof KnolError;
  }

  toJSON() {
    return {
      name: this.name,
      message: this.message,
      statusCode: this.statusCode,
      requestId: this.requestId,
      details: this.details,
    };
  }
}

// ============================================================================
// Client Configuration
// ============================================================================

export interface KnolClientConfig {
  apiKey: string;
  baseUrl?: string;
  timeout?: number;
  retryAttempts?: number;
  retryDelayMs?: number;
}

// ============================================================================
// Request/Response Utilities
// ============================================================================

export interface RequestInit extends globalThis.RequestInit {
  timeout?: number;
}

export interface PaginationOptions {
  limit?: number;
  offset?: number;
}

export interface AuditLogOptions {
  limit?: number;
  offset?: number;
  memory_id?: string;
}
