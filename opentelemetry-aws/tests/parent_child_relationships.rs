//! Integration tests for parent-child span relationships.
//!
//! Tests that parent-child relationships are correctly translated to X-Ray
//! segments and subsegments, including precursor IDs for sibling subsegments.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use opentelemetry::trace::{SpanId, SpanKind};
use opentelemetry_aws::xray_exporter::{SegmentTranslator, XrayExporter};
use opentelemetry_sdk::trace::SpanExporter;

#[tokio::test]
async fn test_parent_child_relationship_basic() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone())
        .with_translator(SegmentTranslator::new().always_nest_subsegments());

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let child_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());

    let parent_span = create_basic_span("parent", SpanKind::Server, trace_id, parent_span_id, None);

    let child_span = create_basic_span(
        "child",
        SpanKind::Client,
        trace_id,
        child_span_id,
        Some(parent_span_id),
    );

    exporter
        .export(vec![parent_span, child_span])
        .await
        .unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    let parent_json = &documents[0];

    // Parent should be a segment (Server span)
    assert_field_eq(parent_json, "name", "parent");
    assert_field_eq(parent_json, "id", "1111111111111111");
    assert_field_exists(parent_json, "trace_id");
    assert_field_not_exists(parent_json, "parent_id");
    assert_field_not_exists(parent_json, "type");

    let subsegments = get_nested_value(parent_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 1, "Should have 1 subsegment");
    let child_json = &subsegments[0];

    // Child have the correct id/name, as it is nested it lacks trace_id, parent_id and type fields
    assert_field_eq(child_json, "name", "child");
    assert_field_eq(child_json, "id", "2222222222222222");
    assert_field_not_exists(child_json, "trace_id");
    assert_field_not_exists(child_json, "parent_id");
    assert_field_not_exists(child_json, "type");
}

#[tokio::test]
async fn test_multiple_children_same_parent() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone())
        .with_translator(SegmentTranslator::new().always_nest_subsegments());

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());

    let parent_span = create_basic_span("parent", SpanKind::Server, trace_id, parent_span_id, None);

    // Create three child spans
    let child1_span = create_basic_span(
        "child1",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x2222222222222222u64.to_be_bytes()),
        Some(parent_span_id),
    );

    let child2_span = create_basic_span(
        "child2",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x3333333333333333u64.to_be_bytes()),
        Some(parent_span_id),
    );

    let child3_span = create_basic_span(
        "child3",
        SpanKind::Internal,
        trace_id,
        SpanId::from_bytes(0x4444444444444444u64.to_be_bytes()),
        Some(parent_span_id),
    );

    exporter
        .export(vec![parent_span, child1_span, child2_span, child3_span])
        .await
        .unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    // Verify parent
    let parent_json = &documents[0];
    assert_field_eq(parent_json, "name", "parent");
    assert_field_eq(parent_json, "id", "1111111111111111");
    assert_field_not_exists(parent_json, "type");

    let subsegments = get_nested_value(parent_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 3, "Should have 3 subsegments");

    // Verify children have the correct id/name, as they are nested they lack trace_id, parent_id and type fields
    for (i, (name, expected_id)) in [
        ("child1", "2222222222222222"),
        ("child2", "3333333333333333"),
        ("child3", "4444444444444444"),
    ]
    .iter()
    .enumerate()
    {
        let child_json = &subsegments[i];
        assert_field_eq(child_json, "name", name);
        assert_field_eq(child_json, "id", expected_id);
        assert_field_not_exists(child_json, "trace_id");
        assert_field_not_exists(child_json, "parent_id");
        assert_field_not_exists(child_json, "type");
    }
}

#[tokio::test]
async fn test_nested_hierarchy_three_levels() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone())
        .with_translator(SegmentTranslator::new().always_nest_subsegments());

    let trace_id = create_valid_trace_id();

    // Level 1: Root span (Server)
    let root_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let root_span = create_basic_span("root", SpanKind::Server, trace_id, root_span_id, None);

    // Level 2: Child of root (Client)
    let child_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let child_span = create_basic_span(
        "child",
        SpanKind::Client,
        trace_id,
        child_span_id,
        Some(root_span_id),
    );

    // Level 3: Grandchild (Internal)
    let grandchild_span_id = SpanId::from_bytes(0x3333333333333333u64.to_be_bytes());
    let grandchild_span = create_basic_span(
        "grandchild",
        SpanKind::Internal,
        trace_id,
        grandchild_span_id,
        Some(child_span_id),
    );

    exporter
        .export(vec![root_span, child_span, grandchild_span])
        .await
        .unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    let root_json = &documents[0];
    // Root should be a segment
    assert_field_eq(root_json, "name", "root");
    assert_field_eq(root_json, "id", "1111111111111111");
    assert_field_exists(root_json, "trace_id");
    assert_field_not_exists(root_json, "parent_id");
    assert_field_not_exists(root_json, "type");

    let subsegments = get_nested_value(root_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 1, "Should have 1 subsegment");
    let child_json = &subsegments[0];

    // Child have the correct id/name, as it is nested it lacks trace_id, parent_id and type fields
    assert_field_eq(child_json, "name", "child");
    assert_field_eq(child_json, "id", "2222222222222222");
    assert_field_not_exists(child_json, "trace_id");
    assert_field_not_exists(child_json, "parent_id");
    assert_field_not_exists(child_json, "type");

    let subsegments = get_nested_value(child_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 1, "Should have 1 subsegment");
    let grandchild_json = &subsegments[0];

    // Grandchild have the correct id/name, as it is nested it lacks trace_id, parent_id and type fields
    assert_field_eq(grandchild_json, "name", "grandchild");
    assert_field_eq(grandchild_json, "id", "3333333333333333");
    assert_field_not_exists(grandchild_json, "trace_id");
    assert_field_not_exists(grandchild_json, "parent_id");
    assert_field_not_exists(grandchild_json, "type");
}

#[tokio::test]
async fn test_sibling_subsegments_ordering() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone())
        .with_translator(SegmentTranslator::new().always_nest_subsegments());

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());

    let parent_span = create_basic_span("parent", SpanKind::Server, trace_id, parent_span_id, None);

    // Create sibling spans with different start times
    use std::time::{Duration, UNIX_EPOCH};

    let mut sibling1 = create_basic_span(
        "sibling1",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x2222222222222222u64.to_be_bytes()),
        Some(parent_span_id),
    );
    sibling1.start_time = UNIX_EPOCH + Duration::from_secs(1700000000);
    sibling1.end_time = UNIX_EPOCH + Duration::from_secs(1700000001);

    let mut sibling2 = create_basic_span(
        "sibling2",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x3333333333333333u64.to_be_bytes()),
        Some(parent_span_id),
    );
    sibling2.start_time = UNIX_EPOCH + Duration::from_secs(1700000002);
    sibling2.end_time = UNIX_EPOCH + Duration::from_secs(1700000003);

    let mut sibling3 = create_basic_span(
        "sibling3",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x4444444444444444u64.to_be_bytes()),
        Some(parent_span_id),
    );
    sibling3.start_time = UNIX_EPOCH + Duration::from_secs(1700000004);
    sibling3.end_time = UNIX_EPOCH + Duration::from_secs(1700000005);

    exporter
        .export(vec![parent_span, sibling1, sibling2, sibling3])
        .await
        .unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    let parent_json = &documents[0];
    assert_field_exists(parent_json, "trace_id");
    assert_field_not_exists(parent_json, "parent_id");
    assert_field_not_exists(parent_json, "type");

    let subsegments = get_nested_value(parent_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 3, "Should have 3 subsegments");

    // Verify all siblings have the correct id/name, as they are nested they lack trace_id, parent_id and type fields
    for (i, (name, expected_id, start_time)) in [
        ("sibling1", "2222222222222222", 1700000000.0),
        ("sibling2", "3333333333333333", 1700000002.0),
        ("sibling3", "4444444444444444", 1700000004.0),
    ]
    .iter()
    .enumerate()
    {
        let sibling_json = &subsegments[i];
        assert_field_eq(sibling_json, "name", name);
        assert_field_eq(sibling_json, "id", expected_id);
        assert_field_eq(sibling_json, "start_time", start_time);
        assert_field_not_exists(sibling_json, "trace_id");
        assert_field_not_exists(sibling_json, "parent_id");
        assert_field_not_exists(sibling_json, "type");
    }

    // First should not have any precursors
    let sibling1_json = &subsegments[0];
    assert_field_not_exists(sibling1_json, "precursor_ids");

    // Second should have first as precursor
    let sibling2_json = &subsegments[1];
    assert_field_eq(sibling2_json, "precursor_ids", ["2222222222222222"]);

    // Third should have first and second as precursors
    let sibling3_json = &subsegments[2];
    assert_field_eq(
        sibling3_json,
        "precursor_ids",
        ["2222222222222222", "3333333333333333"],
    );
}

#[tokio::test]
async fn test_complex_tree_structure() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone())
        .with_translator(SegmentTranslator::new().always_nest_subsegments());

    let trace_id = create_valid_trace_id();

    // Build a complex tree:
    //       root (Server)
    //      /     \
    //   child1  child2 (Client)
    //     |       |
    //    gc1     gc2 (Internal)

    let root_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let child1_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let child2_id = SpanId::from_bytes(0x3333333333333333u64.to_be_bytes());
    let gc1_id = SpanId::from_bytes(0x4444444444444444u64.to_be_bytes());
    let gc2_id = SpanId::from_bytes(0x5555555555555555u64.to_be_bytes());

    let spans = vec![
        create_basic_span("root", SpanKind::Server, trace_id, root_id, None),
        create_basic_span(
            "child1",
            SpanKind::Client,
            trace_id,
            child1_id,
            Some(root_id),
        ),
        create_basic_span(
            "child2",
            SpanKind::Client,
            trace_id,
            child2_id,
            Some(root_id),
        ),
        create_basic_span("gc1", SpanKind::Internal, trace_id, gc1_id, Some(child1_id)),
        create_basic_span("gc2", SpanKind::Internal, trace_id, gc2_id, Some(child2_id)),
    ];

    exporter.export(spans).await.unwrap();

    let documents = mock_exporter.get_documents();
    dbg!(&documents);
    assert_eq!(documents.len(), 1, "Should export 1 document");

    let root_json = &documents[0];
    // Root should be a segment
    assert_field_eq(root_json, "name", "root");
    assert_field_eq(root_json, "id", "1111111111111111");
    assert_field_exists(root_json, "trace_id");
    assert_field_not_exists(root_json, "parent_id");
    assert_field_not_exists(root_json, "type");

    let subsegments = get_nested_value(root_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 2, "Should have 2 subsegments");

    // Child1
    let child1_json = &subsegments[0];
    assert_field_eq(child1_json, "name", "child1");
    assert_field_eq(child1_json, "id", "2222222222222222");
    assert_field_not_exists(child1_json, "trace_id");
    assert_field_not_exists(child1_json, "parent_id");
    assert_field_not_exists(child1_json, "type");

    // Child2
    let child2_json = &subsegments[1];
    assert_field_eq(child2_json, "name", "child2");
    assert_field_eq(child2_json, "id", "3333333333333333");
    assert_field_not_exists(child2_json, "trace_id");
    assert_field_not_exists(child2_json, "parent_id");
    assert_field_not_exists(child2_json, "type");

    // GC1
    let subsegments = get_nested_value(child1_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 1, "Should have 1 subsegment");
    let gc1_json = &subsegments[0];
    assert_field_eq(gc1_json, "name", "gc1");
    assert_field_eq(gc1_json, "id", "4444444444444444");
    assert_field_not_exists(gc1_json, "trace_id");
    assert_field_not_exists(gc1_json, "parent_id");
    assert_field_not_exists(gc1_json, "type");

    // GC2
    let subsegments = get_nested_value(child2_json, "subsegments")
        .and_then(|v| v.as_array())
        .expect("subsegments should be an array");
    assert_eq!(subsegments.len(), 1, "Should have 1 subsegment");
    let gc2_json = &subsegments[0];
    assert_field_eq(gc2_json, "name", "gc2");
    assert_field_eq(gc2_json, "id", "5555555555555555");
    assert_field_not_exists(gc2_json, "trace_id");
    assert_field_not_exists(gc2_json, "parent_id");
    assert_field_not_exists(gc2_json, "type");
}
