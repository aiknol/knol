# Knol Memory Platform SDK

Official TypeScript/JavaScript SDK for the [Knol memory platform](https://knol.ai). Build intelligent applications with semantic memory, graph traversal, and knowledge extraction.

## Features

- **Full TypeScript Support** - Complete type safety with zero external dependencies (beyond dev deps)
- **Memory Operations** - Write, read, search, and manage memories with semantic understanding
- **Graph Traversal** - Navigate entity relationships with depth-limited exploration and path finding
- **Batch Operations** - Efficiently process multiple memories in a single request
- **Query Builder Pattern** - Fluent API for constructing complex searches
- **Webhook Support** - Real-time event notifications for memory and entity changes
- **Retry Logic** - Automatic exponential backoff for transient failures
- **Browser & Node.js** - Works in any JavaScript environment with fetch support
- **Error Handling** - Custom KnolError class with detailed error information

## Installation

```bash
npm install @knol/sdk
```

Or with yarn:

```bash
yarn add @knol/sdk
```

## Quick Start

```typescript
import { KnolClient } from '@knol/sdk';

// Initialize client
const client = new KnolClient({
  apiKey: process.env.KNOL_API_KEY,
});

// Write a memory
const memory = await client.writeMemory({
  content: 'User prefers tea over coffee',
  user_id: 'user-123',
  role: 'assistant',
  kind: 'insight',
});

// Search memories
const results = await client.searchMemory({
  query: 'user preferences',
  user_id: 'user-123',
  limit: 10,
});

// Get an entity and explore relationships
const entity = await client.getEntity('coffee');
const neighbors = await client.getEntityNeighbors('coffee', 'related_to');
```

## Configuration

```typescript
const client = new KnolClient({
  apiKey: process.env.KNOL_API_KEY,        // Required: API key
  baseUrl: 'https://api.knol.ai',          // Optional: API endpoint (default shown)
  timeout: 30000,                          // Optional: Request timeout in ms (default: 30000)
  retryAttempts: 3,                        // Optional: Number of retry attempts (default: 3)
  retryDelayMs: 1000,                      // Optional: Delay between retries in ms (default: 1000)
});
```

## Memory Operations

### Write a Memory

```typescript
const memory = await client.writeMemory({
  content: 'User completed onboarding training',
  user_id: 'user-123',
  role: 'system',
  session_id: 'session-456',
  kind: 'fact',
  importance: 0.8,
  tags: ['onboarding', 'training'],
  metadata: { source: 'lms' },
});
```

### Batch Write Memories

```typescript
const response = await client.batchWriteMemory([
  {
    content: 'First memory',
    user_id: 'user-123',
  },
  {
    content: 'Second memory',
    user_id: 'user-123',
  },
], true); // parallel = true

console.log(`Created: ${response.created.length}, Failed: ${response.failed.length}`);
```

### Search Memories

#### Simple Search

```typescript
const results = await client.searchMemory({
  query: 'user preferences',
  user_id: 'user-123',
  limit: 10,
});

results.results.forEach(memory => {
  console.log(`${memory.content} (score: ${memory.score})`);
});
```

#### Advanced Search with Query Builder

```typescript
const results = await client.searchMemory(
  client
    .searchBuilder()
    .query('team collaboration')
    .userId('user-123')
    .scope(['private', 'team'])
    .kind('interaction')
    .limit(20)
    .minConfidence(0.7)
    .minImportance(0.5)
    .tags(['meeting', 'decision'])
    .applyDecay(true)
    .build()
);
```

#### Temporal Filtering

```typescript
const results = await client.searchMemory({
  query: 'recent updates',
  user_id: 'user-123',
  temporal_filter: {
    recency_days: 7,      // Last 7 days
  },
  limit: 20,
});

// Or with date range
const results2 = await client.searchMemory({
  query: 'historical events',
  temporal_filter: {
    start_date: '2024-01-01',
    end_date: '2024-12-31',
  },
});
```

### Get a Memory

```typescript
const memory = await client.getMemory('memory-id');
console.log(memory.content);
```

### Update a Memory

```typescript
const updated = await client.updateMemory('memory-id', {
  content: 'Updated content',
  importance: 0.9,
  status: 'active',
  tags: ['important'],
});
```

### Delete a Memory

```typescript
await client.deleteMemory('memory-id');
```

### Export Memories

```typescript
const exportResponse = await client.exportMemories({
  user_id: 'user-123',
  scope: 'private',
  format: 'json',
  include_metadata: true,
});

console.log(`Download from: ${exportResponse.url}`);
console.log(`Expires at: ${exportResponse.expires_at}`);
```

### Import Memories

```typescript
const importResponse = await client.importMemories({
  data: [
    {
      content: 'Imported memory 1',
      user_id: 'user-123',
    },
    {
      content: 'Imported memory 2',
      user_id: 'user-123',
    },
  ],
  update_existing: false,
});

console.log(`Imported: ${importResponse.imported}, Skipped: ${importResponse.skipped}`);
```

## Graph Operations

### List Entities

```typescript
const response = await client.listEntities('person', 100);
response.entities.forEach(entity => {
  console.log(`${entity.name} (${entity.entity_type})`);
});
```

### Get Entity Details

```typescript
const entity = await client.getEntity('alice');
console.log(entity.name, entity.description);
```

### Get Entity Edges

```typescript
const edgesResponse = await client.getEntityEdges('alice');
edgesResponse.edges.forEach(edge => {
  console.log(`${edge.source_id} --[${edge.relation_type}]--> ${edge.target_id}`);
});
```

### Get Entity Neighbors

```typescript
const neighbors = await client.getEntityNeighbors(
  'alice',
  'knows',  // relation type
  50        // limit
);

neighbors.neighbors.forEach(neighbor => {
  console.log(neighbor.name);
});
```

### Expand Entity (2-Hop)

```typescript
const expanded = await client.expandEntity('alice');

console.log('Center:', expanded.center.name);
console.log('First hop entities:', expanded.first_hop.entities.length);
console.log('Second hop entities:', expanded.second_hop.entities.length);
```

### Traverse Graph (N-Hop)

```typescript
const traversal = await client.traverseGraph(
  'alice',
  3,    // depth
  200   // limit
);

traversal.paths.forEach(path => {
  const nodeNames = path.nodes.map(n => n.name).join(' -> ');
  console.log(`Path (depth ${path.depth}): ${nodeNames}`);
});
```

### Find Path Between Entities

```typescript
const pathResult = await client.findPath(
  'alice',
  'bob',
  5     // max_depth
);

if (pathResult.found && pathResult.path) {
  const route = pathResult.path.nodes.map(n => n.name).join(' -> ');
  console.log(`Path: ${route} (distance: ${pathResult.distance})`);
} else {
  console.log('No path found');
}
```

## Webhook Operations

### List Webhooks

```typescript
const response = await client.listWebhooks();
response.webhooks.forEach(webhook => {
  console.log(`${webhook.url} - Events: ${webhook.events.join(', ')}`);
});
```

### Create Webhook

```typescript
const webhook = await client.createWebhook({
  url: 'https://your-app.com/webhooks/knol',
  events: ['memory.created', 'memory.updated', 'entity.created'],
  active: true,
});

console.log(`Created webhook: ${webhook.id}`);
```

### Delete Webhook

```typescript
await client.deleteWebhook('webhook-id');
```

## Admin Operations

### Get Tenant Usage

```typescript
const usage = await client.getTenantUsage();
console.log(`Memories: ${usage.memory_count}`);
console.log(`Entities: ${usage.entity_count}`);
console.log(`Storage: ${usage.storage_bytes} bytes`);
console.log(`API calls this month: ${usage.api_calls_month}`);
```

### List Audit Log

```typescript
const auditResponse = await client.listAuditLog(50, 0);
auditResponse.entries.forEach(entry => {
  console.log(`[${entry.timestamp}] ${entry.user_id} - ${entry.action} (${entry.status})`);
});
```

## Error Handling

The SDK provides a custom `KnolError` class for better error handling:

```typescript
import { KnolClient, KnolError } from '@knol/sdk';

const client = new KnolClient({ apiKey: 'sk-...' });

try {
  const memory = await client.writeMemory({
    content: 'Test memory',
    user_id: 'user-123',
  });
} catch (error) {
  if (KnolError.isKnolError(error)) {
    console.error(`Error: ${error.message}`);
    console.error(`Status: ${error.statusCode}`);
    console.error(`Request ID: ${error.requestId}`);
    console.error(`Details:`, error.details);
  } else {
    console.error('Unknown error:', error);
  }
}
```

## Type Safety

The SDK is fully typed with TypeScript. All request and response types are exported:

```typescript
import {
  Memory,
  WriteMemoryRequest,
  SearchMemoryRequest,
  Entity,
  SearchMemoryResponse,
  KnolError,
} from '@knol/sdk';

// Your code will have full autocomplete and type checking
```

## Browser Usage

The SDK works in modern browsers that support `fetch`:

```typescript
import { KnolClient } from '@knol/sdk';

const client = new KnolClient({
  apiKey: 'pk_live_...',  // Use public API key in browser
  baseUrl: 'https://api.knol.ai',
});

// Make requests from the browser
const results = await client.searchMemory({
  query: 'user preferences',
  user_id: 'current-user',
  limit: 10,
});
```

## Node.js Usage

The SDK works in Node.js 16+ with no additional setup:

```typescript
import { KnolClient } from '@knol/sdk';

const client = new KnolClient({
  apiKey: process.env.KNOL_API_KEY,
});

// Use in server-side applications
const memory = await client.writeMemory({
  content: 'Server-side memory',
  user_id: 'service-account',
});
```

## Advanced Examples

### Building a Recommendation System

```typescript
// Find user preferences
const preferences = await client.searchMemory(
  client
    .searchBuilder()
    .query('user preferences')
    .userId('user-123')
    .kind('insight')
    .minImportance(0.7)
    .limit(10)
    .build()
);

// Find related entities
for (const pref of preferences.results) {
  const entities = await client.searchMemory({
    query: pref.content,
    entity_types: ['product', 'service'],
    limit: 5,
  });
  // Use entities for recommendations
}
```

### Mapping Knowledge Graphs

```typescript
// Start with a root entity
const root = await client.getEntity('machine-learning');

// Expand the knowledge graph
const expanded = await client.expandEntity('machine-learning');

// Map relationships
const relationships = new Map();
for (const edge of expanded.first_hop.edges) {
  relationships.set(edge.target_id, edge.relation_type);
}

console.log('Related concepts:', Array.from(relationships.keys()));
```

### Semantic Memory Timeline

```typescript
// Get memories from a time period
const timeline = await client.searchMemory({
  query: 'project progress',
  user_id: 'user-123',
  temporal_filter: {
    start_date: '2024-01-01',
    end_date: '2024-12-31',
  },
  limit: 100,
});

// Sort by creation date
const sorted = timeline.results.sort(
  (a, b) => new Date(a.created_at || 0).getTime() - new Date(b.created_at || 0).getTime()
);

sorted.forEach(memory => {
  console.log(`${memory.created_at}: ${memory.content}`);
});
```

## API Reference

### Memory Types

- **MemoryScope**: `'private' | 'team' | 'organization' | 'public'`
- **MemoryKind**: `'fact' | 'insight' | 'interaction' | 'context' | 'summary'`
- **MemoryStatus**: `'active' | 'archived' | 'deleted'`

### Query Parameters

- **limit**: Maximum number of results (default: 20, max: 1000)
- **min_confidence**: Minimum confidence score (0-1)
- **min_importance**: Minimum importance score (0-1)
- **apply_decay**: Apply temporal decay to results (default: true)
- **graph_depth**: Maximum graph depth for entity exploration (default: 2)

## Performance Tips

1. **Use Batch Operations** - For multiple writes, use `batchWriteMemory` instead of individual calls
2. **Limit Graph Depth** - Use smaller `graph_depth` values to reduce query time
3. **Filter by Scope** - Use scope filtering to narrow search results
4. **Set Appropriate Limits** - Request only the number of results you need
5. **Cache Entities** - Entity data changes infrequently; consider caching

## Contributing

Contributions are welcome! Please submit issues and pull requests to the [GitHub repository](https://github.com/aiknol/knol-sdk-typescript).

## License

MIT

## Support

- Documentation: https://docs.knol.ai
- Issues: https://github.com/aiknol/knol-sdk-typescript/issues
- Email: aiknolcontact@gmail.com
