"""Async Memory Infrastructure API client."""

from __future__ import annotations

import re
import httpx
from typing import Any, Optional
from uuid import UUID
from datetime import datetime

from .client import _validate_id


class AsyncMemoryClient:
    """Async client for the Memory Infrastructure API.

    Usage:
        async with AsyncMemoryClient(api_key="your-key", base_url="http://localhost:8080") as client:
            # Add a memory
            result = await client.add("The user prefers dark mode", user_id="user-123")

            # Search memories
            results = await client.search("What are the user's preferences?", user_id="user-123")

            # Get a specific memory
            memory = await client.get(memory_id)

            # Update a memory
            await client.update(memory_id, content="Updated content")

            # Delete a memory
            await client.delete(memory_id)
    """

    def __init__(
        self,
        api_key: str,
        base_url: str = "http://localhost:8080",
        timeout: float = 30.0,
    ):
        self._base_url = base_url.rstrip("/")
        self._client = httpx.AsyncClient(
            base_url=self._base_url,
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
            timeout=timeout,
        )

    async def close(self):
        """Close the HTTP client."""
        await self._client.aclose()

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        await self.close()

    # ── Memory Operations ──

    async def add(
        self,
        content: str,
        *,
        user_id: Optional[str] = None,
        role: str = "user",
        session_id: Optional[str] = None,
        agent_id: Optional[str] = None,
        metadata: Optional[dict[str, Any]] = None,
    ) -> dict:
        """Add a memory (conversation turn) to the system.

        The content will be stored as an episode and processed asynchronously
        to extract memories, entities, and relationships.

        Args:
            content: The text content to memorize.
            user_id: Optional user identifier for scoping.
            role: Role of the speaker (user/assistant/system/tool).
            session_id: Optional session identifier.
            agent_id: Optional agent identifier.
            metadata: Optional metadata dictionary.

        Returns:
            dict with episode_id and status.
        """
        payload: dict[str, Any] = {"content": content}
        if user_id:
            payload["user_id"] = user_id
        if role:
            payload["role"] = role
        if session_id:
            payload["session_id"] = session_id
        if agent_id:
            payload["agent_id"] = agent_id
        if metadata:
            payload["metadata"] = metadata

        response = await self._client.post("/v1/memory", json=payload)
        response.raise_for_status()
        return response.json()

    async def add_batch(
        self,
        items: list[dict[str, Any]],
    ) -> list[dict]:
        """Add multiple memories in a single request.

        Args:
            items: List of dicts with 'content' and optional 'user_id', 'role', etc.

        Returns:
            List of dicts with episode_id and status for each item.
        """
        response = await self._client.post("/v1/memory/batch", json=items)
        response.raise_for_status()
        return response.json()

    async def search(
        self,
        query: str,
        *,
        user_id: Optional[str] = None,
        scope: Optional[str] = None,
        kind: Optional[str] = None,
        limit: int = 10,
        min_confidence: Optional[float] = None,
        after: Optional[datetime] = None,
        before: Optional[datetime] = None,
        point_in_time: Optional[datetime] = None,
        session_id: Optional[str] = None,
        agent_id: Optional[str] = None,
        tags: Optional[list[str]] = None,
        entity_types: Optional[list[str]] = None,
        min_importance: Optional[float] = None,
        apply_decay: Optional[bool] = None,
        graph_depth: Optional[int] = None,
    ) -> dict:
        """Search memories using hybrid retrieval (vector + graph + temporal).

        The system automatically classifies query intent and routes to the
        optimal retrieval strategy (preference→vector, temporal→graph, etc.).

        Args:
            query: Natural language search query.
            user_id: Scope search to a specific user.
            scope: Filter by scope (user/team/project/agent/org).
            kind: Filter by kind (preference/fact/task/event/relationship).
            limit: Maximum number of results (default 10).
            min_confidence: Minimum confidence threshold (0.0-1.0).
            after: Only return memories after this time.
            before: Only return memories before this time.
            point_in_time: Simulate memory state at this timestamp.
            session_id: Scope search to a specific session.
            agent_id: Scope search to a specific agent.
            tags: Filter by tags.
            entity_types: Filter by entity types.
            min_importance: Minimum importance threshold (0.0-1.0).
            apply_decay: Apply temporal decay to relevance scoring.
            graph_depth: Depth of graph traversal for retrieval.

        Returns:
            dict with results (list of SearchResult), total count, and query_ms.
        """
        payload: dict[str, Any] = {"query": query, "limit": limit}
        if user_id:
            payload["user_id"] = user_id
        if scope:
            payload["scope"] = scope
        if kind:
            payload["kind"] = kind
        if min_confidence is not None:
            payload["min_confidence"] = min_confidence
        if session_id:
            payload["session_id"] = session_id
        if agent_id:
            payload["agent_id"] = agent_id
        if tags:
            payload["tags"] = tags
        if entity_types:
            payload["entity_types"] = entity_types
        if min_importance is not None:
            payload["min_importance"] = min_importance
        if apply_decay is not None:
            payload["apply_decay"] = apply_decay
        if graph_depth is not None:
            payload["graph_depth"] = graph_depth

        temporal_filter: dict[str, str] = {}
        if after:
            temporal_filter["after"] = after.isoformat()
        if before:
            temporal_filter["before"] = before.isoformat()
        if point_in_time:
            temporal_filter["point_in_time"] = point_in_time.isoformat()
        if temporal_filter:
            payload["temporal_filter"] = temporal_filter

        response = await self._client.post("/v1/memory/search", json=payload)
        response.raise_for_status()
        return response.json()

    async def get(self, memory_id: str) -> dict:
        """Get a specific memory by ID.

        Args:
            memory_id: UUID of the memory.

        Returns:
            Full memory object.
        """
        _validate_id(memory_id, "memory_id")
        response = await self._client.get(f"/v1/memory/{memory_id}")
        response.raise_for_status()
        return response.json()

    async def update(
        self,
        memory_id: str,
        *,
        content: Optional[str] = None,
        status: Optional[str] = None,
        importance: Optional[float] = None,
    ) -> dict:
        """Update a memory.

        Args:
            memory_id: UUID of the memory to update.
            content: New content text.
            status: New status (active/superseded/archived/deleted).
            importance: New importance score (0.0-1.0).

        Returns:
            Updated memory confirmation.
        """
        _validate_id(memory_id, "memory_id")
        payload: dict[str, Any] = {}
        if content is not None:
            payload["content"] = content
        if status is not None:
            payload["status"] = status
        if importance is not None:
            payload["importance"] = importance

        response = await self._client.put(f"/v1/memory/{memory_id}", json=payload)
        response.raise_for_status()
        return response.json()

    async def delete(self, memory_id: str, *, permanent: bool = False) -> None:
        """Delete a memory.

        Args:
            memory_id: UUID of the memory to delete.
            permanent: If True, permanently delete (requires Admin role).
                       If False (default), soft delete with ability to restore.
        """
        _validate_id(memory_id, "memory_id")
        params = {}
        if permanent:
            params["permanent"] = "true"
        response = await self._client.delete(f"/v1/memory/{memory_id}", params=params)
        response.raise_for_status()

    async def restore(self, memory_id: str) -> dict:
        """Restore a soft-deleted memory.

        Args:
            memory_id: UUID of the memory to restore.

        Returns:
            dict with id and status.
        """
        _validate_id(memory_id, "memory_id")
        response = await self._client.post(f"/v1/memory/{memory_id}/restore")
        response.raise_for_status()
        return response.json()

    async def export_memories(
        self,
        user_id: str,
        *,
        include_graph: bool = False,
        format: str = "json",
    ) -> dict:
        """Export memories for a user.

        Args:
            user_id: User identifier to export memories for.
            include_graph: Include knowledge graph data in export.
            format: Export format (json/csv).

        Returns:
            dict with export_id, status, and download URL.
        """
        payload: dict[str, Any] = {
            "user_id": user_id,
            "include_graph": include_graph,
            "format": format,
        }
        response = await self._client.post("/v1/memory/export", json=payload)
        response.raise_for_status()
        return response.json()

    async def import_memories(
        self,
        items: list[dict[str, Any]],
        *,
        conflict_strategy: str = "skip",
    ) -> dict:
        """Import memories into the system.

        Args:
            items: List of memory items to import.
            conflict_strategy: Strategy for handling conflicts (skip/merge/overwrite).

        Returns:
            dict with import_id, status, and summary.
        """
        payload: dict[str, Any] = {
            "items": items,
            "conflict_strategy": conflict_strategy,
        }
        response = await self._client.post("/v1/memory/import", json=payload)
        response.raise_for_status()
        return response.json()

    # ── Graph Operations ──

    async def list_entities(
        self,
        *,
        entity_type: Optional[str] = None,
        limit: int = 50,
    ) -> list[dict]:
        """List entities in the knowledge graph.

        Args:
            entity_type: Filter by type (person/org/concept/location/product).
            limit: Maximum results.

        Returns:
            List of entity objects.
        """
        params: dict[str, Any] = {"limit": limit}
        if entity_type:
            params["entity_type"] = entity_type

        response = await self._client.get("/v1/graph/entities", params=params)
        response.raise_for_status()
        return response.json()

    async def get_entity(self, entity_id: str) -> dict:
        """Get a specific entity by ID."""
        _validate_id(entity_id, "entity_id")
        response = await self._client.get(f"/v1/graph/entities/{entity_id}")
        response.raise_for_status()
        return response.json()

    async def get_entity_edges(self, entity_id: str) -> dict:
        """Get all edges (relationships) for an entity."""
        _validate_id(entity_id, "entity_id")
        response = await self._client.get(f"/v1/graph/entities/{entity_id}/edges")
        response.raise_for_status()
        return response.json()

    async def expand_entity(self, entity_id: str) -> dict:
        """Get 2-hop graph expansion from an entity."""
        _validate_id(entity_id, "entity_id")
        response = await self._client.get(f"/v1/graph/entities/{entity_id}/expand")
        response.raise_for_status()
        return response.json()

    async def traverse_entity(
        self,
        entity_id: str,
        *,
        depth: int = 3,
        limit: int = 50,
    ) -> dict:
        """Traverse the knowledge graph from an entity to a given depth.

        Args:
            entity_id: Starting entity ID.
            depth: Maximum traversal depth (default 3).
            limit: Maximum results per level (default 50).

        Returns:
            dict with traversal result tree structure.
        """
        _validate_id(entity_id, "entity_id")
        params: dict[str, Any] = {"depth": depth, "limit": limit}
        response = await self._client.get(
            f"/v1/graph/entities/{entity_id}/traverse", params=params
        )
        response.raise_for_status()
        return response.json()

    async def find_path(
        self,
        from_id: str,
        to_id: str,
        *,
        max_depth: int = 5,
    ) -> dict:
        """Find shortest path between two entities in the knowledge graph.

        Args:
            from_id: Starting entity ID.
            to_id: Target entity ID.
            max_depth: Maximum path depth (default 5).

        Returns:
            dict with path, length, and intermediate entities.
        """
        _validate_id(from_id, "from_id")
        _validate_id(to_id, "to_id")
        params: dict[str, Any] = {"max_depth": max_depth}
        response = await self._client.get(
            f"/v1/graph/path/{from_id}/{to_id}", params=params
        )
        response.raise_for_status()
        return response.json()

    async def get_entity_neighbors(
        self,
        entity_id: str,
        *,
        rel_type: Optional[str] = None,
        limit: int = 20,
    ) -> dict:
        """Get neighboring entities of a specific entity.

        Args:
            entity_id: Entity ID.
            rel_type: Filter by relationship type.
            limit: Maximum results (default 20).

        Returns:
            dict with neighbors and their relationships.
        """
        _validate_id(entity_id, "entity_id")
        params: dict[str, Any] = {"limit": limit}
        if rel_type:
            params["rel_type"] = rel_type

        response = await self._client.get(
            f"/v1/graph/entities/{entity_id}/neighbors", params=params
        )
        response.raise_for_status()
        return response.json()

    # ── Webhook Operations ──

    async def list_webhooks(self) -> list[dict]:
        """List all registered webhooks.

        Returns:
            List of webhook objects with id, url, events, and created_at.
        """
        response = await self._client.get("/v1/webhooks")
        response.raise_for_status()
        return response.json()

    async def create_webhook(
        self,
        url: str,
        events: list[str],
        *,
        secret: Optional[str] = None,
    ) -> dict:
        """Create a new webhook.

        Args:
            url: Webhook URL to receive events.
            events: List of events to subscribe to (e.g. memory.created, entity.updated).
            secret: Optional secret for HMAC signature verification.

        Returns:
            dict with webhook_id, url, and status.
        """
        payload: dict[str, Any] = {"url": url, "events": events}
        if secret:
            payload["secret"] = secret

        response = await self._client.post("/v1/webhooks", json=payload)
        response.raise_for_status()
        return response.json()

    async def delete_webhook(self, webhook_id: str) -> None:
        """Delete a webhook.

        Args:
            webhook_id: ID of the webhook to delete.
        """
        _validate_id(webhook_id, "webhook_id")
        response = await self._client.delete(f"/v1/webhooks/{webhook_id}")
        response.raise_for_status()

    # ── Admin Operations ──

    async def get_usage(self) -> dict:
        """Get current usage and plan information."""
        response = await self._client.get("/v1/admin/tenants")
        response.raise_for_status()
        return response.json()

    async def list_audit_log(
        self,
        *,
        memory_id: Optional[str] = None,
        limit: int = 50,
    ) -> list[dict]:
        """List audit log entries."""
        params: dict[str, Any] = {"limit": limit}
        if memory_id:
            params["memory_id"] = memory_id

        response = await self._client.get("/v1/admin/audit", params=params)
        response.raise_for_status()
        return response.json()

    async def list_policies(self) -> list[dict]:
        """List retention/memory policies.

        Returns:
            List of active policy objects.
        """
        response = await self._client.get("/v1/admin/policies")
        response.raise_for_status()
        return response.json()

    async def create_policy(self, policy: dict[str, Any]) -> dict:
        """Create a memory policy.

        Args:
            policy: Policy definition.

        Returns:
            Created policy object.
        """
        response = await self._client.post("/v1/admin/policies", json=policy)
        response.raise_for_status()
        return response.json()

    # ── Webhook Signature Verification ──

    @staticmethod
    def verify_webhook_signature(
        payload: str,
        signature: str,
        secret: str,
    ) -> bool:
        """Verify a webhook HMAC-SHA256 signature.

        Args:
            payload: Raw request body string.
            signature: Hex-encoded HMAC signature from X-Webhook-Signature header.
            secret: Webhook secret used when creating the webhook.

        Returns:
            True if the signature is valid.
        """
        import hmac
        import hashlib
        expected = hmac.new(
            secret.encode("utf-8"),
            payload.encode("utf-8"),
            hashlib.sha256,
        ).hexdigest()
        return hmac.compare_digest(expected, signature)
