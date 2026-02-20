# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2025-02-19

### Fixed

- **Dockerfile** — Changed `EXPOSE 3000` to `EXPOSE 8080 8081 8082 8083` to match actual service ports.

### Added

- **Startup validation** — All services now validate required environment variables (`DATABASE_URL`, `NATS_URL`, `REDIS_URL`) on boot, with clear error messages referencing `.env.example`.
- **Secret management** — API keys in LLM providers (`AnthropicProvider`, `OpenAiProvider`, `GeminiProvider`) now use `secrecy::Secret<String>` to prevent accidental logging via `Debug` or `Display`.
- **Pre-push safety hook** — Added `scripts/pre-push-safety.sh` to prevent accidental pushes of the full monorepo to the public OSS remote.
- **Security policy** — Added `SECURITY.md` with vulnerability reporting process and best practices.

## [0.1.0] - 2025-02-18

### Added

- **Gateway Service** — API entry point with bearer token auth, RBAC (Admin/Developer/ReadOnly), plan-based rate limiting, and request routing to internal services.
- **Write Service** — Memory ingestion with fast-ACK pattern, SHA256 content deduplication, and async event publishing via NATS JetStream.
- **Retrieve Service** — Adaptive hybrid search combining vector similarity (pgvector), BM25 full-text search, scope cascade, N-hop graph traversal, and Reciprocal Rank Fusion (RRF) scoring.
- **Graph Service** — NATS consumer for write events with LLM-powered entity/relationship extraction, conflict detection, embedding generation, and webhook dispatch.
- **Multi-LLM support** — Anthropic Claude, OpenAI GPT, and Google Gemini providers for memory extraction and verification.
- **Knowledge graph** — Automatic entity and relationship extraction with upsert logic, edge creation, and graph traversal queries.
- **Memory types** — Episodic, semantic, procedural, and working memory with multi-scope support (user, session, agent, team, org).
- **PII redaction** — Built-in detection and configurable redaction for emails, phone numbers, SSNs, credit cards, IP addresses, and dates of birth.
- **Policy engine** — Retention limits, access control by scope, content filtering with blocked keywords, and auto-redaction policies.
- **Conflict detection** — Automatic detection of contradictions and duplicates with configurable resolution strategies (supersede, skip, review, merge).
- **Decay scoring** — Configurable exponential/linear decay so older memories fade while recently accessed ones stay relevant.
- **Webhooks** — Event subscriptions for memory.created, entity.created, edge.created, conflict.detected, and extraction.completed.
- **Database migrations** — Consolidated baseline with pgvector, Row-Level Security, and RBAC tables.
- **SDKs** — Python (sync + async, LangChain + CrewAI integrations), TypeScript (zero dependencies), and MCP Server (Claude Code, Cursor, Windsurf).
- **CI pipeline** — GitHub Actions with cargo check, test, clippy, and format verification.
- **Docker support** — Multi-stage Dockerfile and docker-compose for local development.

[0.1.0]: https://github.com/aiknol/knol/releases/tag/v0.1.0
