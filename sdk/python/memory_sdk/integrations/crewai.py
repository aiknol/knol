"""CrewAI integrations for Knol Memory SDK.

This module provides CrewAI-compatible memory and storage backends:
- KnolCrewMemory: Short-term memory for agents backed by Knol
- KnolCrewStorage: Long-term storage for crew task outputs
"""

from __future__ import annotations

from typing import Any, Optional, List
from abc import ABC

try:
    from crewai.memory.storage import Storage
    from crewai.memory.memory import Memory
except ImportError as e:
    raise ImportError(
        "CrewAI is not installed. Install it with: pip install crewai"
    ) from e

from memory_sdk.client import MemoryClient


class KnolCrewMemory:
    """CrewAI-compatible memory backend powered by Knol.

    Provides short-term memory for agents using Knol's hybrid retrieval,
    supporting semantic search, graph relationships, and temporal queries.

    Attributes:
        client: MemoryClient instance for Knol communication.
        agent_id: Optional agent identifier for scoping.
        user_id: Optional user identifier.

    Usage:
        from memory_sdk.integrations.crewai import KnolCrewMemory
        from memory_sdk.client import MemoryClient
        from crewai import Agent

        client = MemoryClient(api_key="key", base_url="http://localhost:3000")
        memory = KnolCrewMemory(client=client, agent_id="agent-1")

        agent = Agent(
            role="Research Assistant",
            goal="Find relevant information",
            memory=memory
        )
    """

    def __init__(
        self,
        client: MemoryClient,
        agent_id: Optional[str] = None,
        user_id: Optional[str] = None,
    ):
        """Initialize Knol crew memory.

        Args:
            client: MemoryClient instance.
            agent_id: Optional agent identifier for scoping.
            user_id: Optional user identifier.
        """
        self.client = client
        self.agent_id = agent_id
        self.user_id = user_id

    def save(
        self,
        key: str,
        value: Any,
        metadata: Optional[dict[str, Any]] = None,
    ) -> None:
        """Save a memory item to Knol.

        Args:
            key: Memory key/identifier.
            value: Memory value/content to store.
            metadata: Optional metadata dictionary for enrichment.
        """
        try:
            # Convert value to string if needed
            content = str(value)

            # Prepare metadata
            mem_metadata = metadata or {}
            mem_metadata["key"] = key
            mem_metadata["source"] = "crewai"

            self.client.add(
                content=content,
                user_id=self.user_id,
                agent_id=self.agent_id,
                metadata=mem_metadata,
            )
        except Exception as e:
            print(f"Error saving memory to Knol: {e}")

    def search(
        self,
        query: str,
        limit: int = 10,
        min_confidence: Optional[float] = None,
    ) -> List[dict[str, Any]]:
        """Search memories in Knol.

        Args:
            query: Search query string.
            limit: Maximum number of results (default: 10).
            min_confidence: Minimum confidence threshold (0.0-1.0).

        Returns:
            List of search results with content and metadata.
        """
        try:
            result = self.client.search(
                query=query,
                user_id=self.user_id,
                limit=limit,
                min_confidence=min_confidence,
            )

            # Format results for CrewAI compatibility
            formatted_results = []
            for memory in result.get("results", []):
                formatted_results.append({
                    "id": memory.get("id"),
                    "content": memory.get("content", ""),
                    "confidence": memory.get("confidence", 0.0),
                    "metadata": memory.get("metadata", {}),
                })

            return formatted_results
        except Exception as e:
            print(f"Error searching memories in Knol: {e}")
            return []

    def reset(self) -> None:
        """Reset memory.

        Note: This is a no-op as Knol handles memory lifecycle management
        through its own policies and retention rules.
        """
        pass

    def get_context(self, query: str) -> str:
        """Get formatted context for a query.

        Convenience method that searches Knol and returns formatted context
        string suitable for prompt injection.

        Args:
            query: Context query string.

        Returns:
            Formatted context string.
        """
        results = self.search(query, limit=5)
        if not results:
            return ""

        context_lines = []
        for i, result in enumerate(results, 1):
            content = result.get("content", "")
            confidence = result.get("confidence", 0.0)
            context_lines.append(f"- {content} (confidence: {confidence:.2f})")

        return "\n".join(context_lines)


class KnolCrewStorage(Storage):
    """CrewAI-compatible long-term storage backed by Knol.

    Stores and retrieves crew task outputs and other long-term data using Knol's
    persistent storage, supporting search, filtering, and metadata enrichment.

    Attributes:
        client: MemoryClient instance for Knol communication.
        user_id: Optional user identifier for scoping.
        agent_id: Optional agent identifier for scoping.

    Usage:
        from memory_sdk.integrations.crewai import KnolCrewStorage
        from memory_sdk.client import MemoryClient
        from crewai import Crew

        client = MemoryClient(api_key="key", base_url="http://localhost:3000")
        storage = KnolCrewStorage(client=client, user_id="user-123")

        crew = Crew(agents=[agent1, agent2], tasks=[task1, task2])
        crew.memory_storage = storage
    """

    def __init__(
        self,
        client: MemoryClient,
        user_id: Optional[str] = None,
        agent_id: Optional[str] = None,
    ):
        """Initialize Knol crew storage.

        Args:
            client: MemoryClient instance.
            user_id: Optional user identifier.
            agent_id: Optional agent identifier.
        """
        self.client = client
        self.user_id = user_id
        self.agent_id = agent_id

    def save(self, item: dict[str, Any]) -> None:
        """Save a task output or item to Knol.

        Stores task output with metadata for later retrieval and analysis.

        Args:
            item: Dictionary containing task output and metadata.
                Expected keys: task_id, output, agent_id, etc.
        """
        try:
            # Extract key fields
            task_id = item.get("task_id", "")
            output = item.get("output", "")
            agent_id = item.get("agent_id") or self.agent_id

            # Prepare metadata
            metadata = {
                "task_id": task_id,
                "storage_type": "task_output",
                **{k: v for k, v in item.items()
                   if k not in ["output", "task_id", "agent_id"]},
            }

            # Store in Knol
            self.client.add(
                content=str(output),
                user_id=self.user_id,
                agent_id=agent_id,
                metadata=metadata,
            )
        except Exception as e:
            print(f"Error saving to Knol storage: {e}")

    def search(
        self,
        query: str,
        limit: int = 10,
        score_threshold: Optional[float] = None,
    ) -> List[dict[str, Any]]:
        """Search stored items in Knol.

        Retrieves task outputs and other stored items matching the query.

        Args:
            query: Search query string.
            limit: Maximum number of results (default: 10).
            score_threshold: Minimum confidence threshold (0.0-1.0).

        Returns:
            List of matching items with content and metadata.
        """
        try:
            result = self.client.search(
                query=query,
                user_id=self.user_id,
                limit=limit,
                min_confidence=score_threshold,
            )

            # Format results
            formatted_results = []
            for memory in result.get("results", []):
                formatted_results.append({
                    "id": memory.get("id"),
                    "content": memory.get("content", ""),
                    "score": memory.get("confidence", 0.0),
                    "metadata": memory.get("metadata", {}),
                })

            return formatted_results
        except Exception as e:
            print(f"Error searching Knol storage: {e}")
            return []

    def get(self, item_id: str) -> Optional[dict[str, Any]]:
        """Retrieve a specific stored item by ID.

        Args:
            item_id: ID of the item to retrieve.

        Returns:
            Item dictionary with content and metadata, or None if not found.
        """
        try:
            memory = self.client.get(item_id)
            return {
                "id": memory.get("id"),
                "content": memory.get("content", ""),
                "metadata": memory.get("metadata", {}),
            }
        except Exception as e:
            print(f"Error retrieving item from Knol storage: {e}")
            return None

    def delete(self, item_id: str) -> None:
        """Delete a stored item from Knol.

        Args:
            item_id: ID of the item to delete.
        """
        try:
            self.client.delete(item_id)
        except Exception as e:
            print(f"Error deleting item from Knol storage: {e}")

    def list(
        self,
        limit: int = 50,
        agent_id: Optional[str] = None,
    ) -> List[dict[str, Any]]:
        """List stored items.

        Returns a list of stored items, optionally filtered by agent.

        Args:
            limit: Maximum number of items to return (default: 50).
            agent_id: Optional agent filter.

        Returns:
            List of stored items with content and metadata.
        """
        try:
            # Use a broad search to get recent items
            result = self.client.search(
                query="*",  # Broad query
                user_id=self.user_id,
                limit=limit,
            )

            formatted_results = []
            for memory in result.get("results", []):
                # Filter by agent if specified
                if agent_id:
                    mem_agent = memory.get("metadata", {}).get("agent_id")
                    if mem_agent != agent_id:
                        continue

                formatted_results.append({
                    "id": memory.get("id"),
                    "content": memory.get("content", ""),
                    "metadata": memory.get("metadata", {}),
                })

            return formatted_results
        except Exception as e:
            print(f"Error listing Knol storage: {e}")
            return []

    def clear(self) -> None:
        """Clear all stored items.

        Note: This is a no-op as Knol handles storage lifecycle management
        through its own policies and retention rules.
        """
        pass
