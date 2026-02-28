<p align="center">
  <h1 align="center">Knol OSS</h1>
  <p align="center">Open-source memory infrastructure for AI applications.<br/>Give your agents persistent, searchable, context-aware memory.</p>
</p>

<p align="center">
  <a href="https://github.com/aiknol/knol/actions"><img src="https://github.com/aiknol/knol/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/aiknol/knol"><img src="https://img.shields.io/badge/rust-1.77+-orange.svg" alt="Rust"></a>
</p>

---

## What is Knol?

Knol is a memory layer for AI agents and LLM-powered applications. Instead of losing context between conversations, your agents can **remember**, **search**, and **reason** over past interactions.

Write a memory in plain text. Knol automatically extracts entities, builds a knowledge graph, detects conflicts with existing memories, and makes everything searchable via vector + full-text hybrid retrieval.

### Key Features

- **Hybrid Search** — Vector similarity + BM25 full-text with Reciprocal Rank Fusion (RRF)
- **Knowledge Graph** — Automatic entity and relationship extraction from conversations
- **Multi-Scope Memory** — User, session, agent, team, and org-level scoping
- **Memory Types** — Episodic, semantic, procedural, and working memory
- **PII Redaction** — Built-in detection and redaction for emails, phones, SSNs, credit cards, and more
- **Conflict Detection** — Automatic detection of contradictions and duplicates
- **Decay Scoring** — Older memories gracefully fade; recently accessed ones stay relevant
- **Policy Engine** — Retention limits, access control, content filtering, and auto-redaction
- **Webhooks** — Subscribe to memory events (created, updated, conflicts detected)
- **Multi-LLM Support** — Anthropic Claude, OpenAI GPT, and Google Gemini for extraction
- **RBAC** — Role-based API keys (Admin, Developer, ReadOnly)
- **Rate Limiting** — Plan-based rate limits via Redis sliding window

## Architecture

Knol is a set of microservices written in Rust, connected via NATS JetStream for async processing:

```
┌─────────────┐     ┌───────────────┐     ┌──────────────────┐
│   Client     │────▶│   Gateway     │────▶│  Write Service   │
│  (SDK/API)   │     │  (port 3000)  │     │   (port 8081)    │
└─────────────┘     │               │     │                  │
                    │  Auth, RBAC   │     │  Fast ACK +      │
                    │  Rate Limit   │     │  NATS publish     │
                    │  Routing      │     └────────┬─────────┘
                    │               │              │ NATS JetStream
                    │               │     ┌────────▼─────────┐
                    │               │     │  Graph Service    │
                    │               │     │   (port 8083)     │
                    │               │     │                  │
                    │               │     │  LLM Extraction  │
                    │               │     │  Entity/Edge     │
                    │               │     │  Conflict Detect │
                    │               │     │  Embeddings      │
                    │               │     │  Webhooks        │
                    │               │     └──────────────────┘
                    │               │
                    │               │     ┌──────────────────┐
                    │               │────▶│ Retrieve Service  │
                    │               │     │   (port 8082)     │
                    └───────────────┘     │                  │
                                          │  Vector Search   │
                                          │  BM25 FTS        │
                                          │  Graph Traversal │
                                          │  RRF Fusion      │
                                          │  Decay Scoring   │
                                          └──────────────────┘

Infrastructure: PostgreSQL (pgvector) · Redis · NATS JetStream · MinIO
```

## Quick Start

### Prerequisites

- Docker and Docker Compose
- An LLM API key (Anthropic, OpenAI, or Google)

### 1. Clone and start infrastructure

```bash
git clone https://github.com/aiknol/knol.git
cd knol/knol-oss

# Start PostgreSQL, Redis, NATS, and MinIO
docker compose up -d
```

### 2. Configure environment

```bash
cp .env.example .env
# Edit .env — at minimum, set your LLM_API_KEY
```

### 3. Run the services

**With Docker (recommended):**

```bash
docker build -t knol-oss .
docker run --env-file .env --network host knol-oss
```

**From source (for development):**

```bash
# Install Rust 1.77+
cargo build --workspace
cargo run --bin service-gateway
```

### 4. Write your first memory

```bash
curl -X POST http://localhost:3000/v1/memory \
  -H "Authorization: Bearer $KNOL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"content": "User prefers dark mode and uses VS Code", "role": "user"}'
```

### 5. Search memories

```bash
curl -X POST http://localhost:3000/v1/memory/search \
  -H "Authorization: Bearer $KNOL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "What editor does the user prefer?"}'
```

## SDKs

| SDK | Location | Description |
|-----|----------|-------------|
| **Python** | [`sdk/python`](sdk/python) | Sync and async clients, LangChain + CrewAI integrations |
| **TypeScript** | [`sdk/typescript`](sdk/typescript) | Zero-dependency client for Node.js and browsers |
| **MCP Server** | [`sdk/mcp`](sdk/mcp) | Model Context Protocol server for Claude Code, Cursor, Windsurf |

### TypeScript Example

```typescript
import { KnolClient } from '@knol/sdk';

const knol = new KnolClient({ apiKey: 'your_api_key' });

await knol.memory.write({ content: 'User likes hiking and photography' });

const results = await knol.memory.search({ query: 'hobbies' });
```

### Python Example

```python
from memory_sdk import MemoryClient

client = MemoryClient(api_key="your_api_key")

client.write("User prefers Python over JavaScript")

results = client.search("programming language preferences")
```

## API Reference

### Memory Operations

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `POST` | `/v1/memory` | Developer+ | Write a memory |
| `POST` | `/v1/memory/batch` | Developer+ | Batch write memories |
| `POST` | `/v1/memory/search` | ReadOnly+ | Search memories |
| `GET` | `/v1/memory/:id` | ReadOnly+ | Get a specific memory |
| `PUT` | `/v1/memory/:id` | Developer+ | Update a memory |
| `DELETE` | `/v1/memory/:id` | Developer+ | Delete a memory |
| `POST` | `/v1/memory/export` | ReadOnly+ | Export memories |
| `POST` | `/v1/memory/import` | Developer+ | Import memories |

### Graph Operations

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/v1/graph/entities` | ReadOnly+ | List entities |
| `GET` | `/v1/graph/entities/:id` | ReadOnly+ | Get entity details |
| `GET` | `/v1/graph/entities/:id/edges` | ReadOnly+ | Get entity edges |
| `GET` | `/v1/graph/entities/:id/neighbors` | ReadOnly+ | Get entity neighbors |
| `GET` | `/v1/graph/entities/:id/traverse` | ReadOnly+ | N-hop graph traversal |
| `GET` | `/v1/graph/path/:from/:to` | ReadOnly+ | Find path between entities |

### Admin Operations

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `POST` | `/v1/webhooks` | Admin | Create webhook |
| `DELETE` | `/v1/webhooks/:id` | Admin | Delete webhook |
| `GET` | `/v1/admin/tenants` | Admin | Get tenant info |
| `GET` | `/v1/admin/audit` | Admin | View audit log |
| `GET` | `/v1/admin/policies` | Admin | List policies |
| `POST` | `/v1/admin/policies` | Admin | Create policy |

### Health & Metrics

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/health` | None | Health check |
| `GET` | `/metrics` | None | Prometheus metrics |

## Services

| Service | Default Port | Role |
|---------|-------------|------|
| **Gateway** | 3000 | Auth, routing, rate limiting |
| **Write** | 8081 | Memory ingestion, dedup, NATS publish |
| **Retrieve** | 8082 | Hybrid search, graph traversal, scoring |
| **Graph** | 8083 | LLM extraction, entity/edge upsert, webhooks |

## Configuration

Knol uses a three-tier configuration system: database (`system_config` table) > environment variables > compiled defaults. See [`.env.example`](.env.example) for all available options.

## Public Release Check

Before publishing this repository, run:

```bash
./scripts/public-readiness.sh
```

This verifies formatting, linting, tests, current-tree secret scan, and git-history high-confidence secret checks, including strict service-graph linting via:

- `cargo clippy -p service-graph --all-targets -- -D warnings`

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and PR guidelines.

## Security

See [SECURITY.md](SECURITY.md) for reporting vulnerabilities.

## License

Apache License 2.0 — see [LICENSE](LICENSE) for details.
