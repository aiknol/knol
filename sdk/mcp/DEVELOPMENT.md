# Development Guide

## Project Structure

```
knol-mcp/
├── src/
│   └── index.ts          # Main MCP server implementation
├── dist/                 # Compiled JavaScript (generated)
├── package.json          # Project metadata and dependencies
├── tsconfig.json         # TypeScript configuration
├── .gitignore            # Git ignore rules
├── .npmignore            # npm ignore rules
├── README.md             # User documentation
└── DEVELOPMENT.md        # This file
```

## Setup for Development

1. **Clone or download the project**

```bash
cd /path/to/knol-mcp
```

2. **Install dependencies**

```bash
npm install
```

3. **Build the project**

```bash
npm run build
```

## Development Workflow

### Watch Mode

For development with automatic recompilation:

```bash
npm run dev
```

This will watch for changes in `src/` and recompile TypeScript files automatically.

### Testing Locally

1. Start a local Knol API server (or use a test server)

2. Set environment variables:

```bash
export KNOL_API_KEY="test-key-12345"
export KNOL_API_URL="http://localhost:3000"
export KNOL_USER_ID="test-user"
```

3. Run the server:

```bash
npm run build
npm start
```

The server will start on stdin/stdout for MCP communication.

## MCP Protocol Overview

The Knol MCP server implements the Model Context Protocol specification:

### Message Types

1. **Tool Listing** - Client requests available tools
   - Server returns list of tools with schemas

2. **Tool Call** - Client invokes a tool with parameters
   - Server processes request and returns result

3. **Resource Listing** - Client requests available resources
   - Server returns list of resources

4. **Resource Read** - Client requests a resource
   - Server returns resource content

### Implementation Details

The server uses:
- `@modelcontextprotocol/sdk` - Official MCP SDK
- `StdioServerTransport` - Standard input/output transport
- `Server` class - Main MCP server instance

## Adding New Tools

To add a new tool to the server:

1. Add tool definition to the `tools` array in `src/index.ts`:

```typescript
{
  name: "knol_new_tool",
  description: "Description of what the tool does",
  inputSchema: {
    type: "object",
    properties: {
      param1: {
        type: "string",
        description: "Parameter description"
      }
    },
    required: ["param1"]
  }
}
```

2. Add handler in the `tools/call` request handler:

```typescript
case "knol_new_tool": {
  const param1 = (args as Record<string, unknown>).param1;
  result = await makeRequest("/v1/new-endpoint", {
    param: param1
  });
  break;
}
```

3. Rebuild and test:

```bash
npm run build
npm start
```

## Adding New Resources

To add a new resource:

1. Add resource definition to the `resources` array:

```typescript
{
  uri: "knol://resource-name",
  name: "Resource Name",
  description: "What this resource provides",
  mimeType: "application/json"
}
```

2. Add handler in the `resources/read` request handler:

```typescript
if (uri === "knol://resource-name") {
  // Fetch and return resource content
  return { contents: [...] };
}
```

## Error Handling

The server handles errors by:

1. Catching exceptions in try-catch blocks
2. Returning error messages with `isError: true` flag
3. Providing meaningful error messages to the client

Common error scenarios:

- Invalid API key → 401 error from Knol API
- Missing required parameters → Handler error
- Network issues → Fetch error
- Malformed requests → Type errors

## Type Safety

The project uses strict TypeScript settings:

- `strict: true` - Enables all strict type checking
- `noImplicitAny: true` - Disallows implicit any types
- `noUnusedLocals: true` - Errors on unused variables
- `noUnusedParameters: true` - Errors on unused parameters

When adding code, ensure all types are explicit and correct.

## Building for Production

1. Ensure all tests pass (if applicable)
2. Build the project:

```bash
npm run build
```

3. Verify dist/index.js exists and is executable:

```bash
ls -la dist/index.js
```

4. Test the built version:

```bash
KNOL_API_KEY="test" node dist/index.js
```

## Publishing to npm

To publish to npm:

1. Update version in `package.json`
2. Build the project: `npm run build`
3. Create a git tag: `git tag v0.1.0`
4. Publish: `npm publish`

The `.npmignore` file ensures only necessary files are included.

## Debugging

### Enable Debug Logging

The MCP SDK may support debug output. Set environment variables:

```bash
DEBUG=* node dist/index.js
```

### Test Individual Endpoints

Use curl to test the Knol API directly:

```bash
curl -H "Authorization: Bearer YOUR_KEY" \
  http://localhost:3000/v1/memory/search \
  -d '{"query":"test","user_id":"user1","limit":5}' \
  -H "Content-Type: application/json"
```

### TypeScript Errors

If you encounter TypeScript errors, check:

1. All imports are correct
2. Function signatures match their usage
3. Type annotations are complete
4. No implicit any types

Run type check:

```bash
npx tsc --noEmit
```

## Dependencies

### Main Dependencies

- `@modelcontextprotocol/sdk` - Official Model Context Protocol implementation

### Dev Dependencies

- `typescript` - TypeScript compiler
- `@types/node` - Node.js type definitions

### Why These?

- MCP SDK provides standardized protocol implementation
- TypeScript ensures type safety and better IDE support
- Node.js types enable better development experience

## Environment Variables

The server respects these environment variables:

| Variable | Purpose | Default |
|----------|---------|---------|
| `KNOL_API_KEY` | Bearer token for API auth | Required |
| `KNOL_API_URL` | Knol API base URL | http://localhost:3000 |
| `KNOL_USER_ID` | Default user for operations | default |

Set them before running the server or in the MCP client configuration.

## Troubleshooting

### Build Errors

1. Ensure Node.js 18+ is installed: `node --version`
2. Clear node_modules: `rm -rf node_modules && npm install`
3. Check TypeScript version: `npx tsc --version`

### Runtime Errors

1. Verify environment variables are set correctly
2. Check Knol API is running and accessible
3. Verify API key has correct permissions
4. Check API response in error message

### MCP Integration Issues

1. Verify server starts without errors: `npm start`
2. Check MCP client configuration points to correct path
3. Ensure environment variables are available to client
4. Verify stdio transport is used (not TCP)

## Performance Considerations

The server is designed to be lightweight:

- Minimal dependencies (only MCP SDK)
- Efficient HTTP client (uses native fetch)
- No persistent state (stateless design)
- Fast startup time
- Low memory footprint

For high-volume use, consider:

- Connection pooling to Knol API
- Response caching
- Request batching
- Connection limits

## Security Notes

- API keys are passed via environment variables
- Never log sensitive information (API keys, user IDs, etc.)
- All HTTP requests use HTTPS in production
- Input validation is performed by the Knol API
- The server acts as a simple proxy to the API

## Future Enhancements

Potential improvements:

- Support for Knol API pagination
- Batch operations for multiple memories
- Streaming responses for large result sets
- Local caching layer
- Request retry logic with exponential backoff
- Metrics and monitoring support
- Configuration file support (instead of env vars)
- Multi-user workspace support
