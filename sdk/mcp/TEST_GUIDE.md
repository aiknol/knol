# Testing Guide

This document explains how to test the Knol MCP server locally.

## Prerequisites

- Node.js 18+
- Server built: `npm run build`
- Knol API server running locally or accessible remotely
- Valid API key

## Unit Testing Setup

### 1. Install Testing Dependencies

```bash
npm install --save-dev jest @types/jest ts-jest
```

### 2. Create jest.config.js

```javascript
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  roots: ['<rootDir>/tests'],
  testMatch: ['**/__tests__/**/*.ts', '**/?(*.)+(spec|test).ts'],
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json', 'node'],
};
```

### 3. Add Test Script to package.json

```json
{
  "scripts": {
    "test": "jest",
    "test:watch": "jest --watch",
    "test:coverage": "jest --coverage"
  }
}
```

## Manual Testing

### Test 1: Server Starts Successfully

```bash
KNOL_API_KEY="test-key" npm start &
sleep 2
kill %1
```

Expected output: Server starts without errors.

### Test 2: Environment Variable Validation

```bash
npm start
```

Expected output: Error about missing KNOL_API_KEY.

### Test 3: Tools List Request

Create `test-tools-list.json`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/list",
  "params": {}
}
```

Send the request:

```bash
cat test-tools-list.json | \
  KNOL_API_KEY="test-key" npm start
```

Expected: Response with tool definitions for all 7 tools.

### Test 4: API Connection Test

Test if the server can reach the Knol API:

```bash
KNOL_API_KEY="sk_test_actual_key" \
KNOL_API_URL="http://localhost:3000" \
KNOL_USER_ID="test-user" \
npm start
```

Create `test-search.json`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_search",
    "arguments": {
      "query": "test",
      "limit": 1
    }
  }
}
```

Send:

```bash
cat test-search.json | \
  KNOL_API_KEY="sk_test_actual_key" \
  KNOL_API_URL="http://localhost:3000" \
  npm start
```

Expected: Response from Knol API (success or API error).

## Integration Testing

### Test 1: Full Memory Lifecycle

```bash
KNOL_API_KEY="your-key" \
KNOL_API_URL="https://api.knol.io" \
KNOL_USER_ID="test-user" \
npm start
```

Execute these requests in sequence:

1. **Store a memory** (knol_remember):

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_remember",
    "arguments": {
      "content": "Test memory for integration testing"
    }
  }
}
```

Note the returned memory ID.

2. **Retrieve the memory** (knol_get):

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "knol_get",
    "arguments": {
      "memory_id": "mem_xyz123"
    }
  }
}
```

3. **Update the memory** (knol_update):

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "knol_update",
    "arguments": {
      "memory_id": "mem_xyz123",
      "content": "Updated test memory",
      "importance": 8
    }
  }
}
```

4. **Search for memories** (knol_search):

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "knol_search",
    "arguments": {
      "query": "integration testing",
      "limit": 5
    }
  }
}
```

5. **Delete the memory** (knol_delete):

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "tools/call",
  "params": {
    "name": "knol_delete",
    "arguments": {
      "memory_id": "mem_xyz123"
    }
  }
}
```

Expected: All operations succeed in sequence.

### Test 2: Knowledge Graph Operations

1. **List entities** (knol_entities):

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_entities",
    "arguments": {
      "entity_type": "technology",
      "limit": 20
    }
  }
}
```

2. **Get entity neighbors** (knol_entity_neighbors):

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "knol_entity_neighbors",
    "arguments": {
      "entity_id": "ent_123",
      "limit": 10
    }
  }
}
```

### Test 3: Resource Access

Request the recent memories resource:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "resources/list",
  "params": {}
}
```

Expected: Response includes knol://recent resource.

Then read it:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "resources/read",
  "params": {
    "uri": "knol://recent"
  }
}
```

Expected: Response with JSON array of recent memories.

## Error Testing

### Test 1: Invalid API Key

```bash
KNOL_API_KEY="invalid_key" npm start
```

Send any request. Expected: 401 Unauthorized error.

### Test 2: Invalid Memory ID

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_get",
    "arguments": {
      "memory_id": "nonexistent_id"
    }
  }
}
```

Expected: 404 Not Found error.

### Test 3: Missing Required Parameters

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_search",
    "arguments": {}
  }
}
```

Expected: Error about missing required query parameter.

### Test 4: Unknown Tool

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_nonexistent",
    "arguments": {}
  }
}
```

Expected: Error about unknown tool.

### Test 5: Unknown Resource

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "resources/read",
  "params": {
    "uri": "knol://nonexistent"
  }
}
```

Expected: Error about unknown resource.

## Performance Testing

### Test 1: Large Query Results

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_search",
    "arguments": {
      "query": "common term",
      "limit": 1000
    }
  }
}
```

Check response time and memory usage.

### Test 2: Rapid Requests

```bash
for i in {1..100}; do
  echo "Request $i"
  curl -X POST http://localhost:3000/v1/memory/search \
    -H "Authorization: Bearer YOUR_KEY" \
    -H "Content-Type: application/json" \
    -d '{"query":"test","limit":5}'
done
```

Monitor for performance degradation.

### Test 3: Large Metadata

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "knol_remember",
    "arguments": {
      "content": "Test with large metadata",
      "metadata": {
        "large_field": "x".repeat(10000)
      }
    }
  }
}
```

## Stress Testing

### Concurrent Requests

Using Apache Bench:

```bash
ab -n 100 -c 10 \
  -H "Authorization: Bearer YOUR_KEY" \
  -H "Content-Type: application/json" \
  -p request.json \
  http://localhost:3000/v1/memory/search
```

### Load Testing with Artillery

Create `load-test.yml`:

```yaml
config:
  target: "http://localhost:3000"
  phases:
    - duration: 60
      arrivalRate: 10
scenarios:
  - name: "Knol API Load Test"
    flow:
      - post:
          url: "/v1/memory/search"
          json:
            query: "test"
            limit: 5
          headers:
            Authorization: "Bearer YOUR_KEY"
```

Run:

```bash
npx artillery run load-test.yml
```

## Local Development Testing

### Watch Mode with Testing

```bash
npm run dev &
npm run test:watch &
```

### Code Coverage

```bash
npm run test:coverage
```

View coverage report:

```bash
open coverage/lcov-report/index.html
```

## CI/CD Testing

### GitHub Actions Example

Create `.github/workflows/test.yml`:

```yaml
name: Test

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v3

      - uses: actions/setup-node@v3
        with:
          node-version: '18'

      - run: npm install

      - run: npm run build

      - run: npm test

      - run: npm run test:coverage

      - uses: codecov/codecov-action@v3
```

## Docker Testing

### Build Test Image

```dockerfile
FROM node:18-alpine

WORKDIR /app
COPY . .

RUN npm install
RUN npm run build

ENTRYPOINT ["npm", "test"]
```

Build and run:

```bash
docker build -t knol-mcp-test .
docker run -e KNOL_API_KEY="test" knol-mcp-test
```

## Debugging

### Enable Verbose Logging

```bash
export DEBUG=*
npm start
```

### Node Inspector

```bash
node --inspect dist/index.js
```

Then open Chrome DevTools at `chrome://inspect`.

### Console Logging

Add logging to `src/index.ts`:

```typescript
console.log("Tool called:", name, args);
```

Rebuild and test:

```bash
npm run build
KNOL_API_KEY="test" npm start | tee server.log
```

## Test Checklist

- [ ] Server starts without KNOL_API_KEY → Error
- [ ] Server starts with KNOL_API_KEY → Success
- [ ] Tools list returns 7 tools
- [ ] knol_remember stores memory
- [ ] knol_get retrieves memory
- [ ] knol_update modifies memory
- [ ] knol_delete removes memory
- [ ] knol_search finds memories
- [ ] knol_entities lists entities
- [ ] knol_entity_neighbors shows relationships
- [ ] knol://recent resource returns recent memories
- [ ] Invalid API key returns 401
- [ ] Invalid memory ID returns 404
- [ ] Unknown tool returns error
- [ ] Unknown resource returns error
- [ ] Large queries handle gracefully
- [ ] Concurrent requests succeed
- [ ] Type checking passes: `npx tsc --noEmit`

## Reporting Issues

When reporting test failures, include:

1. Your test command
2. Expected output
3. Actual output
4. Server logs
5. Environment variables (without secrets)
6. Node version: `node --version`
7. npm version: `npm --version`

Example:

```
Test: knol_search with large limit
Command: knol_search (query: "test", limit: 1000)
Expected: Response with up to 1000 results
Actual: Timeout after 30 seconds
Node: v18.0.0
npm: 8.0.0
Logs: [paste relevant logs]
```
