//! Integration tests for memory-common types.

use chrono::Utc;
use memory_common::*;
use uuid::Uuid;

#[test]
fn test_full_write_search_roundtrip_types() {
    // Simulate a write request
    let write_req = MemoryWriteRequest {
        content: "I prefer using Rust for backend development".to_string(),
        role: Some("user".to_string()),
        user_id: Some(Uuid::new_v4()),
        session_id: Some("session-abc".to_string()),
        agent_id: None,
        metadata: Some(serde_json::json!({"topic": "programming"})),
    };

    // Serialize and deserialize
    let json = serde_json::to_string(&write_req).unwrap();
    let deserialized: MemoryWriteRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.content, write_req.content);

    // Create a write event (what would be published to NATS)
    let event = MemoryWriteEvent {
        episode_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        user_id: write_req.user_id,
        content: write_req.content.clone(),
        role: "user".to_string(),
        session_id: write_req.session_id.clone(),
        agent_id: None,
        metadata: write_req.metadata.clone().unwrap_or(serde_json::json!({})),
        timestamp: Utc::now(),
    };

    let event_json = serde_json::to_vec(&event).unwrap();
    let event_back: MemoryWriteEvent = serde_json::from_slice(&event_json).unwrap();
    assert_eq!(event_back.episode_id, event.episode_id);

    // Simulate extraction result
    let extraction = ExtractionResult {
        memories: vec![ExtractedMemory {
            content: "User prefers Rust for backend development".to_string(),
            kind: "preference".to_string(),
            confidence: 0.95,
            importance: 0.8,
            tags: vec!["programming".to_string(), "rust".to_string()],
            source_quote: None,
            source_offset_start: None,
            source_offset_end: None,
        }],
        entities: vec![ExtractedEntity {
            name: "Rust".to_string(),
            entity_type: "concept".to_string(),
            summary: Some("Systems programming language".to_string()),
            attributes: Some(serde_json::json!({"category": "language"})),
        }],
        relationships: vec![ExtractedRelationship {
            source_entity: "User".to_string(),
            target_entity: "Rust".to_string(),
            rel_type: "prefers".to_string(),
            properties: Some(serde_json::json!({"context": "backend"})),
            weight: Some(0.95),
        }],
    };

    let ext_json = serde_json::to_string(&extraction).unwrap();
    let ext_back: ExtractionResult = serde_json::from_str(&ext_json).unwrap();
    assert_eq!(ext_back.memories[0].confidence, 0.95);

    // Simulate search request
    let search_req = MemorySearchRequest {
        query: "What programming language does the user prefer?".to_string(),
        user_id: write_req.user_id,
        scope: Some("user".to_string()),
        kind: Some("preference".to_string()),
        limit: Some(5),
        min_confidence: Some(0.5),
        temporal_filter: None,
        session_id: None,
        agent_id: None,
        tags: None,
        entity_types: None,
        min_importance: None,
        apply_decay: None,
        graph_depth: None,
    };

    let search_json = serde_json::to_string(&search_req).unwrap();
    let search_back: MemorySearchRequest = serde_json::from_str(&search_json).unwrap();
    assert_eq!(search_back.limit, Some(5));

    // Simulate search response
    let search_response = MemorySearchResponse {
        results: vec![SearchResult {
            memory: MemoryItem {
                id: Uuid::new_v4(),
                tenant_id: event.tenant_id,
                user_id: write_req.user_id,
                scope: "user".to_string(),
                kind: "preference".to_string(),
                content: "User prefers Rust for backend development".to_string(),
                content_json: None,
                confidence: 0.95,
                importance: 0.8,
                status: "active".to_string(),
                valid_from: Utc::now(),
                valid_to: None,
                event_time: Some(Utc::now()),
                ingested_at: Utc::now(),
                source_episode_id: Some(event.episode_id),
                created_by: "system".to_string(),
                tags: vec!["programming".to_string()],
                metadata: serde_json::json!({}),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                deleted_at: None,
            },
            score: 0.92,
            vector_score: Some(0.88),
            graph_score: Some(0.95),
            related_entities: vec![],
        }],
        total: 1,
        query_ms: 12,
    };

    let resp_json = serde_json::to_string(&search_response).unwrap();
    let resp_back: MemorySearchResponse = serde_json::from_str(&resp_json).unwrap();
    assert_eq!(resp_back.total, 1);
    assert_eq!(resp_back.results[0].score, 0.92);
}

#[test]
fn test_temporal_filter_variants() {
    // All None
    let filter = TemporalFilter {
        after: None,
        before: None,
        point_in_time: None,
    };
    let json = serde_json::to_string(&filter).unwrap();
    assert!(json.contains("null"));

    // Point in time
    let pit_filter = TemporalFilter {
        after: None,
        before: None,
        point_in_time: Some(Utc::now()),
    };
    let json2 = serde_json::to_string(&pit_filter).unwrap();
    assert!(json2.contains("point_in_time"));

    // Range filter
    let range_filter = TemporalFilter {
        after: Some(Utc::now()),
        before: Some(Utc::now()),
        point_in_time: None,
    };
    let json3 = serde_json::to_string(&range_filter).unwrap();
    let back: TemporalFilter = serde_json::from_str(&json3).unwrap();
    assert!(back.after.is_some());
    assert!(back.before.is_some());
}
