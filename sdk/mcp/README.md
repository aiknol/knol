# Knol MCP Server

A Model Context Protocol (MCP) server for integrating the Knol memory platform with AI coding tools like Claude Code, Cursor, and Windsurf. This allows AI assistants to persistently store and retrieve memories across sessions, building a contextual knowledge graph.

## Features

- **Memory Management**: Store, retrieve, search, update, and delete memories
- **Semantic Search**: Find memories using semantic queries with configurable limits and filters
- **Knowledge Graph**: Explore entities, relationships, and entity neighbors in the knowledge base
- **Session Support**: Group memories by session for context-aware retrieval
- **Metadata Support**: Attach custom metadata to memories for rich context
- **Importance Tracking**: Mark memories with importance scores for prioritization
- **Graph Traversal**: Explore multi-hop entity relationships with configurable depth

## Installation

### Prerequisites

- Node.js 18+
- npm or yarn
- Running Knol API server

### Step 1: Build the server

```bash
cd /path/to/knol-mcp
npm install
npm run build
```

The compiled server will be available at `dist/index.js`.

## Configuration

The server reads configuration from environment variables:

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `KNOL_API_KEY` | Yes | - | Bearer token for Knol API authentication |
| `KNOL_API_URL` | No | `http://localhost:3000` | Knol API endpoint URL |
| `KNOL_USER_ID` | No | `default` | Default user ID for operations |

## MCP Tools

### knol_remember
Store a memory in Knol.

**Parameters:**
- `content` (string, required): The memory content to store
- `user_id` (string, optional): User ID (uses KNOL_USER_ID if not provided)
- `session_id` (string, optional): Session ID for grouping related memories
- `metadata` (object, optional): Additional metadata to attach

**Example:**
```json
{
  "content": "User prefers TypeScript for backend development",
  "session_id": "session-123",
  "metadata": { "category": "preferences", "confidence": 0.95 }
}
```

### knol_search
Search for memories using semantic queries.

**Parameters:**
- `query` (string, required): Search query string
- `user_id` (string, optional): Filter by user ID
- `limit` (number, optional): Maximum results to return (default: 5)
- `kind` (string, optional): Filter by memory kind/type
- `graph_depth` (number, optional): Entity graph traversal depth (default: 0)

**Example:**
```json
{
  "query": "TypeScript backend preferences",
  "limit": 10,
  "graph_depth": 1
}
```

### knol_get
Retrieve a specific memory by ID.

**Parameters:**
- `memory_id` (string, required): The memory ID to retrieve

**Example:**
```json
{
  "memory_id": "mem-abc123"
}
```

### knol_update
Update an existing memory.

**Parameters:**
- `memory_id` (string, required): The memory ID to update
- `content` (string, optional): Updated memory content
- `importance` (number, optional): Importance score (0-10)

**Example:**
```json
{
  "memory_id": "mem-abc123",
  "content": "Updated content",
  "importance": 8
}
```

### knol_delete
Delete a memory.

**Parameters:**
- `memory_id` (string, required): The memory ID to delete

**Example:**
```json
{
  "memory_id": "mem-abc123"
}
```

### knol_entities
List knowledge graph entities.

**Parameters:**
- `entity_type` (string, optional): Filter by entity type
- `limit` (number, optional): Maximum entities to return (default: 20)

**Example:**
```json
{
  "entity_type": "technology",
  "limit": 50
}
```

### knol_entity_neighbors
Get related entities for a specific entity.

**Parameters:**
- `entity_id` (string, required): The entity ID to explore
- `limit` (number, optional): Maximum neighbors to return (default: 10)

**Example:**
```json
{
  "entity_id": "ent-typescript",
  "limit": 20
}
```

## MCP Resources

### knol://recent
Returns the 10 most recent memories for the configured user.

This resource is automatically available and can be accessed to quickly retrieve recent context.

## Integration Guides

### Claude Code

Add to your Claude Code configuration file (`~/.config/Claude Code/mcp.json` or in project settings):

```json
{
  "mcpServers": {
    "knol": {
      "command": "node",
      "args": ["/path/to/knol-mcp/dist/index.js"],
      "env": {
        "KNOL_API_KEY": "your-api-key-here",
        "KNOL_API_URL": "https://api.knol.example.com",
        "KNOL_USER_ID": "your-user-id"
      }
    }
  }
}
```

### Cursor

Add to your Cursor settings (`.cursor/settings.json`):

```json
{
  "mcpServers": {
    "knol": {
      "command": "node",
      "args": ["/path/to/knol-mcp/dist/index.js"],
      "env": {
        "KNOL_API_KEY": "your-api-key-here",
        "KNOL_API_URL": "https://api.knol.example.com",
        "KNOL_USER_ID": "your-user-id"
      }
    }
  }
}
```

### Windsurf

Add to your Windsurf configuration (`.windsurf/mcp.json`):

```json
{
  "mcpServers": {
    "knol": {
      "command": "node",
      "args": ["/path/to/knol-mcp/dist/index.js"],
      "env": {
        "KNOL_API_KEY": "your-api-key-here",
        "KNOL_API_URL": "https://api.knol.example.com",
        "KNOL_USER_ID": "your-user-id"
      }
    }
  }
}
```

## Alternative: npm Installation

If published to npm, you can also use the npx shorthand:

```json
{
  "mcpServers": {
    "knol": {
      "command": "npx",
      "args": ["@knol/mcp-server"],
      "env": {
        "KNOL_API_KEY": "your-api-key-here",
        "KNOL_API_URL": "https://api.knol.example.com",
        "KNOL_USER_ID": "your-user-id"
      }
    }
  }
}
```

## Development

### Building

```bash
npm run build
```

### Watch Mode

```bash
npm run dev
```

### Testing

To test the server locally, set up environment variables and run:

```bash
export KNOL_API_KEY="test-key"
export KNOL_API_URL="http://localhost:3000"
npm run build
npm start
```

## Architecture

The MCP server is built on the `@modelcontextprotocol/sdk` and implements:

- **Tools**: Seven main tools for memory and entity operations
- **Resources**: One resource (`knol://recent`) for quick access to recent memories
- **Transport**: Standard Input/Output (stdio) for integration with MCP clients

The server:
1. Reads configuration from environment variables
2. Makes authenticated HTTP requests to the Knol API
3. Exposes tools and resources through the MCP protocol
4. Returns formatted results to the MCP client

## API Reference

For detailed Knol API documentation, see: https://knol.example.com/docs

### Endpoints Used

- `POST /v1/memory` — Store a memory
- `POST /v1/memory/search` — Search memories
- `GET /v1/memory/:id` — Get a specific memory
- `PUT /v1/memory/:id` — Update a memory
- `DELETE /v1/memory/:id` — Delete a memory
- `GET /v1/graph/entities` — List entities
- `GET /v1/graph/entities/:id/neighbors` — Get entity relationships

## Error Handling

The server handles errors gracefully and returns error messages in the MCP response format. Common errors include:

- **401 Unauthorized**: Invalid or missing API key
- **404 Not Found**: Memory or entity not found
- **500 Server Error**: Knol API server error

All errors are returned with an `isError: true` flag in the MCP response.

## Security

- API keys are passed via environment variables and never exposed in logs
- All communication uses Bearer token authentication
- The server validates inputs before sending to the Knol API
- Configure appropriate KNOL_API_URL for your deployment

## Contributing

Contributions are welcome! Please submit pull requests to the Knol repository.

## License

MIT License - See LICENSE file for details

## Support

For issues, questions, or feature requests, please open an issue on GitHub or contact the Knol team.
