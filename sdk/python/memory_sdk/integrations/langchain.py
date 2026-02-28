"""LangChain integrations for Knol Memory SDK.

This module provides LangChain-compatible memory backends:
- KnolMemory: Conversational memory backed by Knol
- KnolChatMessageHistory: Chat history backed by Knol
- KnolRetriever: Document retriever backed by Knol
"""

from __future__ import annotations

from typing import Any, List, Optional
from datetime import datetime

try:
    from langchain.memory.base import BaseMemory
    from langchain.schema import BaseMessage, HumanMessage, AIMessage, SystemMessage
    from langchain.retrievers.base import BaseRetriever
    from langchain.schema import Document
    from langchain.memory.chat_message_histories.base import BaseChatMessageHistory
except ImportError as e:
    raise ImportError(
        "LangChain is not installed. Install it with: pip install langchain"
    ) from e

from memory_sdk.client import MemoryClient


class KnolMemory(BaseMemory):
    """LangChain memory backend powered by Knol.

    Stores conversation history and relevant context in Knol, supporting:
    - Vector-based semantic search for relevant context
    - Graph-based relationship querying
    - Temporal filtering of memories
    - Automatic memory consolidation via Knol

    Attributes:
        memory_key: Key to access memory in LangChain variables (default: "history").
        input_key: Key for input in conversation (default: "input").
        output_key: Key for output in conversation (default: "output").
        client: MemoryClient instance for Knol communication.
        user_id: Optional user identifier for memory scoping.
        session_id: Optional session identifier.
        return_messages: Whether to return messages as Message objects (default: False).

    Usage:
        from memory_sdk.integrations.langchain import KnolMemory
        from memory_sdk.client import MemoryClient
        from langchain.chat_models import ChatOpenAI

        client = MemoryClient(api_key="key", base_url="http://localhost:3000")
        memory = KnolMemory(
            client=client,
            user_id="user-123",
            session_id="session-456"
        )

        llm = ChatOpenAI()
        chain = ConversationChain(llm=llm, memory=memory)
        response = chain.run(input="Tell me something interesting")
    """

    memory_key: str = "history"
    input_key: str = "input"
    output_key: str = "output"
    client: MemoryClient
    user_id: Optional[str] = None
    session_id: Optional[str] = None
    return_messages: bool = False

    class Config:
        """Pydantic config."""
        arbitrary_types_allowed = True

    @property
    def memory_variables(self) -> List[str]:
        """Return memory variables."""
        return [self.memory_key]

    def load_memory_variables(self, inputs: dict[str, Any]) -> dict[str, Any]:
        """Load memory variables from Knol based on input.

        Searches Knol for context relevant to the latest input using semantic
        search and returns formatted memories.

        Args:
            inputs: Dictionary containing at least the input_key.

        Returns:
            Dictionary with memory_key containing formatted memory string or messages.
        """
        # Extract the latest input query
        query = inputs.get(self.input_key, "")
        if not query:
            return {self.memory_key: "" if not self.return_messages else []}

        try:
            # Search Knol for relevant memories
            search_result = self.client.search(
                query=query,
                user_id=self.user_id,
                limit=10,
            )

            results = search_result.get("results", [])

            if not results:
                return {self.memory_key: "" if not self.return_messages else []}

            if self.return_messages:
                # Convert to LangChain Message objects
                messages = []
                for result in results:
                    content = result.get("content", "")
                    role = result.get("metadata", {}).get("role", "user")
                    if role == "assistant":
                        messages.append(AIMessage(content=content))
                    elif role == "system":
                        messages.append(SystemMessage(content=content))
                    else:
                        messages.append(HumanMessage(content=content))
                return {self.memory_key: messages}
            else:
                # Format as string
                formatted_memories = []
                for i, result in enumerate(results, 1):
                    content = result.get("content", "")
                    confidence = result.get("confidence", 0.0)
                    formatted_memories.append(
                        f"{i}. [confidence: {confidence:.2f}] {content}"
                    )
                return {self.memory_key: "\n".join(formatted_memories)}

        except Exception as e:
            # Log error and return empty memory to allow conversation to continue
            print(f"Error loading memory from Knol: {e}")
            return {self.memory_key: "" if not self.return_messages else []}

    def save_context(self, inputs: dict[str, str], outputs: dict[str, str]) -> None:
        """Save conversation context to Knol.

        Stores the input/output pair as separate memory episodes in Knol.

        Args:
            inputs: Dictionary with input_key containing user input.
            outputs: Dictionary with output_key containing model output.
        """
        input_str = inputs.get(self.input_key, "")
        output_str = outputs.get(self.output_key, "")

        try:
            if input_str:
                self.client.add(
                    content=input_str,
                    user_id=self.user_id,
                    session_id=self.session_id,
                    role="user",
                    metadata={"type": "conversation"},
                )

            if output_str:
                self.client.add(
                    content=output_str,
                    user_id=self.user_id,
                    session_id=self.session_id,
                    role="assistant",
                    metadata={"type": "conversation"},
                )
        except Exception as e:
            print(f"Error saving context to Knol: {e}")

    def clear(self) -> None:
        """Clear memory.

        Note: This is a no-op as Knol handles memory retention and lifecycle
        management through its own policies.
        """
        pass


class KnolChatMessageHistory(BaseChatMessageHistory):
    """LangChain chat history backed by Knol.

    Stores and retrieves chat messages from Knol, providing a persistent
    conversation history backend.

    Attributes:
        client: MemoryClient instance for Knol communication.
        user_id: User identifier for scoping messages.
        session_id: Optional session identifier.

    Usage:
        from memory_sdk.integrations.langchain import KnolChatMessageHistory
        from memory_sdk.client import MemoryClient
        from langchain.memory import ConversationBufferMemory

        client = MemoryClient(api_key="key", base_url="http://localhost:3000")
        history = KnolChatMessageHistory(
            client=client,
            user_id="user-123",
            session_id="session-456"
        )

        memory = ConversationBufferMemory(chat_memory=history)
    """

    def __init__(
        self,
        client: MemoryClient,
        user_id: str,
        session_id: Optional[str] = None,
    ):
        """Initialize Knol chat message history.

        Args:
            client: MemoryClient instance.
            user_id: User identifier for scoping.
            session_id: Optional session identifier.
        """
        self.client = client
        self.user_id = user_id
        self.session_id = session_id

    def add_message(self, message: BaseMessage) -> None:
        """Add a message to Knol.

        Args:
            message: LangChain BaseMessage to store.
        """
        try:
            # Determine role from message type
            if isinstance(message, HumanMessage):
                role = "user"
            elif isinstance(message, AIMessage):
                role = "assistant"
            elif isinstance(message, SystemMessage):
                role = "system"
            else:
                role = "user"

            self.client.add(
                content=message.content,
                user_id=self.user_id,
                session_id=self.session_id,
                role=role,
                metadata={"type": "chat_message"},
            )
        except Exception as e:
            print(f"Error adding message to Knol: {e}")

    def add_messages(self, messages: List[BaseMessage]) -> None:
        """Add multiple messages to Knol.

        Args:
            messages: List of LangChain messages to store.
        """
        for message in messages:
            self.add_message(message)

    @property
    def messages(self) -> List[BaseMessage]:
        """Retrieve messages from Knol.

        Returns recent messages stored in Knol for this user/session.

        Returns:
            List of LangChain BaseMessage objects.
        """
        try:
            # Search for recent chat messages
            result = self.client.search(
                query="",  # Empty query to get recent messages
                user_id=self.user_id,
                kind="message",
                limit=50,
            )

            messages = []
            for memory in result.get("results", []):
                content = memory.get("content", "")
                role = memory.get("metadata", {}).get("role", "user")

                if role == "assistant":
                    messages.append(AIMessage(content=content))
                elif role == "system":
                    messages.append(SystemMessage(content=content))
                else:
                    messages.append(HumanMessage(content=content))

            return messages
        except Exception as e:
            print(f"Error retrieving messages from Knol: {e}")
            return []

    def clear(self) -> None:
        """Clear message history.

        Note: This is a no-op as Knol handles memory retention and lifecycle
        management through its own policies.
        """
        pass


class KnolRetriever(BaseRetriever):
    """LangChain retriever backed by Knol.

    Uses Knol's hybrid retrieval (vector + graph + temporal) to find
    relevant documents for a given query.

    Attributes:
        client: MemoryClient instance for Knol communication.
        user_id: Optional user identifier for scoping.
        kind: Optional memory kind filter (preference/fact/task/event/relationship).
        min_confidence: Minimum confidence threshold (0.0-1.0).
        return_metadata: Whether to include Knol metadata in Document.metadata.

    Usage:
        from memory_sdk.integrations.langchain import KnolRetriever
        from memory_sdk.client import MemoryClient

        client = MemoryClient(api_key="key", base_url="http://localhost:3000")
        retriever = KnolRetriever(client=client, user_id="user-123")

        docs = retriever.get_relevant_documents("What are user preferences?")
        for doc in docs:
            print(doc.page_content)
    """

    def __init__(
        self,
        client: MemoryClient,
        user_id: Optional[str] = None,
        kind: Optional[str] = None,
        min_confidence: Optional[float] = None,
        return_metadata: bool = True,
    ):
        """Initialize Knol retriever.

        Args:
            client: MemoryClient instance.
            user_id: Optional user identifier for scoping.
            kind: Optional memory kind filter.
            min_confidence: Minimum confidence threshold.
            return_metadata: Whether to include Knol metadata.
        """
        super().__init__()
        self.client = client
        self.user_id = user_id
        self.kind = kind
        self.min_confidence = min_confidence
        self.return_metadata = return_metadata

    def _get_relevant_documents(self, query: str) -> List[Document]:
        """Retrieve relevant documents from Knol.

        Args:
            query: Search query string.

        Returns:
            List of LangChain Document objects with page_content and metadata.
        """
        try:
            result = self.client.search(
                query=query,
                user_id=self.user_id,
                kind=self.kind,
                min_confidence=self.min_confidence,
                limit=10,
            )

            documents = []
            for memory in result.get("results", []):
                content = memory.get("content", "")
                memory_id = memory.get("id", "")
                confidence = memory.get("confidence", 0.0)

                metadata = {}
                if self.return_metadata:
                    metadata = {
                        "memory_id": memory_id,
                        "confidence": confidence,
                        "kind": memory.get("kind"),
                        "created_at": memory.get("created_at"),
                    }

                doc = Document(page_content=content, metadata=metadata)
                documents.append(doc)

            return documents
        except Exception as e:
            print(f"Error retrieving documents from Knol: {e}")
            return []

    async def _aget_relevant_documents(self, query: str) -> List[Document]:
        """Async version of _get_relevant_documents.

        Args:
            query: Search query string.

        Returns:
            List of LangChain Document objects.
        """
        # For now, use sync version. Can be optimized with async client later.
        return self._get_relevant_documents(query)
