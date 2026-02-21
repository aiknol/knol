#!/usr/bin/env node

import {
  Server,
  StdioServerTransport,
  Tool,
  Resource,
  TextContent,
  ResourceContents,
} from "@modelcontextprotocol/sdk";

// Configuration from environment
const KNOL_API_URL = process.env.KNOL_API_URL || "http://localhost:8080";
const KNOL_API_KEY = process.env.KNOL_API_KEY;
const KNOL_USER_ID = process.env.KNOL_USER_ID || "default";

if (!KNOL_API_KEY) {
  console.error("Error: KNOL_API_KEY environment variable is required");
  process.exit(1);
}

// Type definitions
interface KnolMemory {
  id: string;
  content: string;
  user_id: string;
  role?: string;
  session_id?: string;
  agent_id?: string;
  metadata?: Record<string, unknown>;
  created_at?: string;
  updated_at?: string;
  importance?: number;
  status?: string;
  confidence?: number;
}

interface KnolEntity {
  id: string;
  name: string;
  type: string;
  properties?: Record<string, unknown>;
  created_at?: string;
  updated_at?: string;
}

interface KnolSearchResult {
  memories: KnolMemory[];
  total: number;
  query: string;
}

// HTTP utility functions
async function makeRequest(
  method: string,
  endpoint: string,
  body?: unknown
): Promise<unknown> {
  const url = `${KNOL_API_URL}${endpoint}`;
  const headers: HeadersInit = {
    "Content-Type": "application/json",
    Authorization: `Bearer ${KNOL_API_KEY}`,
  };

  // AbortController with 30s timeout to prevent hung requests
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30_000);

  const options: RequestInit = {
    method,
    headers,
    signal: controller.signal,
  };

  if (body) {
    options.body = JSON.stringify(body);
  }

  let response: Response;
  try {
    response = await fetch(url, options);
  } finally {
    clearTimeout(timeout);
  }

  if (!response.ok) {
    const error = await response.text();
    throw new Error(`Knol API error (${response.status}): ${error}`);
  }

  const contentType = response.headers.get("content-type");
  if (contentType?.includes("application/json")) {
    return await response.json();
  }

  return await response.text();
}

// Initialize MCP Server
const server = new Server({
  name: "knol-mcp",
  version: "0.1.0",
});

// Tools
const tools: Tool[] = [
  {
    name: "knol_remember",
    description:
      "Store a memory in Knol. Memories are searchable, taggable, and can be connected to knowledge graph entities.",
    inputSchema: {
      type: "object",
      properties: {
        content: {
          type: "string",
          description: "The memory content to store",
        },
        user_id: {
          type: "string",
          description: "User ID (defaults to configured KNOL_USER_ID)",
        },
        session_id: {
          type: "string",
          description: "Session ID for grouping related memories",
        },
        metadata: {
          type: "object",
          description: "Additional metadata to attach to the memory",
        },
      },
      required: ["content"],
    },
  },
  {
    name: "knol_search",
    description:
      "Search for memories in Knol. Supports semantic search, filtering by kind, confidence, and graph depth traversal.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "Search query string",
        },
        user_id: {
          type: "string",
          description: "Filter by user ID",
        },
        limit: {
          type: "number",
          description: "Maximum results to return (default: 5)",
        },
        kind: {
          type: "string",
          description: "Filter by memory kind/type",
        },
        graph_depth: {
          type: "number",
          description: "Entity graph traversal depth (default: 0)",
        },
      },
      required: ["query"],
    },
  },
  {
    name: "knol_get",
    description: "Retrieve a specific memory by ID",
    inputSchema: {
      type: "object",
      properties: {
        memory_id: {
          type: "string",
          description: "The memory ID to retrieve",
        },
      },
      required: ["memory_id"],
    },
  },
  {
    name: "knol_update",
    description: "Update an existing memory",
    inputSchema: {
      type: "object",
      properties: {
        memory_id: {
          type: "string",
          description: "The memory ID to update",
        },
        content: {
          type: "string",
          description: "Updated memory content",
        },
        importance: {
          type: "number",
          description: "Importance score (0-10)",
        },
      },
      required: ["memory_id"],
    },
  },
  {
    name: "knol_delete",
    description: "Delete a memory",
    inputSchema: {
      type: "object",
      properties: {
        memory_id: {
          type: "string",
          description: "The memory ID to delete",
        },
      },
      required: ["memory_id"],
    },
  },
  {
    name: "knol_entities",
    description:
      "List knowledge graph entities. Useful for discovering structured information in the knowledge base.",
    inputSchema: {
      type: "object",
      properties: {
        entity_type: {
          type: "string",
          description: "Filter by entity type",
        },
        limit: {
          type: "number",
          description: "Maximum entities to return (default: 20)",
        },
      },
    },
  },
  {
    name: "knol_entity_neighbors",
    description:
      "Get related entities for a specific entity. Useful for exploring knowledge graph relationships.",
    inputSchema: {
      type: "object",
      properties: {
        entity_id: {
          type: "string",
          description: "The entity ID to explore",
        },
        limit: {
          type: "number",
          description: "Maximum neighbors to return (default: 10)",
        },
      },
      required: ["entity_id"],
    },
  },
];

/**
 * Sanitize an ID parameter to prevent path traversal attacks.
 * Only allows UUID-format strings (hex + hyphens) and rejects anything
 * containing path separators, dots, or other unsafe characters.
 */
function sanitizeId(id: unknown, paramName: string): string {
  const str = String(id || "");
  // UUIDs: 8-4-4-4-12 hex characters with hyphens
  const uuidRegex = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
  if (!uuidRegex.test(str)) {
    throw new Error(`Invalid ${paramName}: must be a valid UUID`);
  }
  return str;
}

// Tool handlers
server.setRequestHandler("tools/list", async () => {
  return { tools };
});

server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;

  try {
    let result: unknown;

    switch (name) {
      case "knol_remember": {
        const userId = (args as Record<string, unknown>).user_id || KNOL_USER_ID;
        result = await makeRequest("POST", "/v1/memory", {
          content: (args as Record<string, unknown>).content,
          user_id: userId,
          session_id: (args as Record<string, unknown>).session_id,
          metadata: (args as Record<string, unknown>).metadata,
        });
        break;
      }

      case "knol_search": {
        const userId = (args as Record<string, unknown>).user_id || KNOL_USER_ID;
        result = await makeRequest("POST", "/v1/memory/search", {
          query: (args as Record<string, unknown>).query,
          user_id: userId,
          limit: (args as Record<string, unknown>).limit || 5,
          kind: (args as Record<string, unknown>).kind,
          graph_depth: (args as Record<string, unknown>).graph_depth,
        });
        break;
      }

      case "knol_get": {
        const memoryId = sanitizeId((args as Record<string, unknown>).memory_id, "memory_id");
        result = await makeRequest("GET", `/v1/memory/${memoryId}`);
        break;
      }

      case "knol_update": {
        const memoryId = sanitizeId((args as Record<string, unknown>).memory_id, "memory_id");
        const updateBody: Record<string, unknown> = {};
        if ((args as Record<string, unknown>).content) {
          updateBody.content = (args as Record<string, unknown>).content;
        }
        if ((args as Record<string, unknown>).importance !== undefined) {
          updateBody.importance = (args as Record<string, unknown>).importance;
        }
        result = await makeRequest("PUT", `/v1/memory/${memoryId}`, updateBody);
        break;
      }

      case "knol_delete": {
        const memoryId = sanitizeId((args as Record<string, unknown>).memory_id, "memory_id");
        result = await makeRequest("DELETE", `/v1/memory/${memoryId}`);
        break;
      }

      case "knol_entities": {
        const params = new URLSearchParams();
        if ((args as Record<string, unknown>).entity_type) {
          params.append(
            "entity_type",
            (args as Record<string, unknown>).entity_type as string
          );
        }
        if ((args as Record<string, unknown>).limit) {
          params.append(
            "limit",
            String((args as Record<string, unknown>).limit)
          );
        }
        const queryString = params.toString();
        const endpoint = queryString ? `/v1/graph/entities?${queryString}` : "/v1/graph/entities";
        result = await makeRequest("GET", endpoint);
        break;
      }

      case "knol_entity_neighbors": {
        const entityId = sanitizeId((args as Record<string, unknown>).entity_id, "entity_id");
        const limit = (args as Record<string, unknown>).limit || 10;
        const endpoint = `/v1/graph/entities/${entityId}/neighbors?limit=${limit}`;
        result = await makeRequest("GET", endpoint);
        break;
      }

      default:
        throw new Error(`Unknown tool: ${name}`);
    }

    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(result, null, 2),
        },
      ],
    };
  } catch (error) {
    const errorMessage =
      error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: "text",
          text: `Error: ${errorMessage}`,
        },
      ],
      isError: true,
    };
  }
});

// Resources
const resources: Resource[] = [
  {
    uri: "knol://recent",
    name: "Recent Memories",
    description: "The 10 most recent memories for the configured user",
    mimeType: "application/json",
  },
];

server.setRequestHandler("resources/list", async () => {
  return { resources };
});

server.setRequestHandler("resources/read", async (request) => {
  const { uri } = request.params;

  if (uri === "knol://recent") {
    try {
      const result = await makeRequest("POST", "/v1/memory/search", {
        query: "*",
        user_id: KNOL_USER_ID,
        limit: 10,
      });

      const contents: ResourceContents[] = [
        {
          uri: "knol://recent",
          mimeType: "application/json",
          text: JSON.stringify(result, null, 2),
        },
      ];

      return { contents };
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      return {
        contents: [
          {
            uri: "knol://recent",
            mimeType: "text/plain",
            text: `Error fetching recent memories: ${errorMessage}`,
          },
        ],
      };
    }
  }

  throw new Error(`Unknown resource: ${uri}`);
});

// Start server
const transport = new StdioServerTransport();
server.connect(transport);
