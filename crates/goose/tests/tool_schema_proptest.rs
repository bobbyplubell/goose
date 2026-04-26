/// Property-based tests for validate_tool_schemas.
///
/// Strategy: generate arbitrary JSON Schema property shapes, wrap them in a
/// realistic tool definition, run through validate_tool_schemas, then assert
/// two invariants:
///
///   1. Structural validity — jsonschema::meta::validate accepts the output.
///      This is the official JSON Schema meta-schema oracle; any output that
///      would cause a strict provider (e.g. Kimi) to return 400 will fail here.
///
///   2. No combinator corruption — anyOf/oneOf/allOf properties must not have
///      type/properties/required injected alongside them, which would produce
///      a contradictory schema.
use goose::providers::formats::openai::validate_tool_schemas;
use proptest::prelude::*;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Schema shape strategies
// ---------------------------------------------------------------------------

/// Leaf schemas — no nesting, all real-world variants we've seen break providers.
fn primitive_schema() -> impl Strategy<Value = Value> {
    prop_oneof![
        // bare scalars
        Just(json!({"type": "string"})),
        Just(json!({"type": "integer"})),
        Just(json!({"type": "number"})),
        Just(json!({"type": "boolean"})),
        Just(json!({"type": "null"})),
        // scalars with description (common in MCP tools)
        Just(json!({"type": "string",  "description": "a string"})),
        Just(json!({"type": "integer", "description": "an integer"})),
        Just(json!({"type": "boolean", "description": "a flag"})),
        // string constraints
        Just(json!({"type": "string", "format": "date-time"})),
        Just(json!({"type": "string", "format": "uri"})),
        Just(json!({"type": "string", "format": "uuid"})),
        Just(json!({"type": "string", "minLength": 1})),
        Just(json!({"type": "string", "minLength": 1, "maxLength": 100})),
        Just(json!({"type": "string", "pattern": "^[a-z]+$"})),
        Just(json!({"type": "string", "default": "hello"})),
        // numeric constraints
        Just(json!({"type": "integer", "minimum": 0})),
        Just(json!({"type": "integer", "minimum": 0, "maximum": 100})),
        Just(json!({"type": "integer", "exclusiveMinimum": 0})),
        Just(json!({"type": "number",  "minimum": 0.0, "maximum": 1.0})),
        // enum / const
        Just(json!({"enum": ["a", "b", "c"]})),
        Just(json!({"enum": [1, 2, 3]})),
        Just(json!({"enum": ["a", "b"], "description": "one of these"})),
        Just(json!({"const": "fixed"})),
        Just(json!({"const": 42})),
        // empty schema (accepts anything — valid JSON Schema)
        Just(json!({})),
        Just(json!({"description": "anything goes"})),
        // title only
        Just(json!({"title": "MyField"})),
        Just(json!({"title": "MyField", "description": "with title"})),
    ]
}

/// $ref schemas — bare and with various siblings (the previous Kimi bug).
fn ref_schema() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(json!({"$ref": "#/definitions/MyType"})),
        Just(json!({"$ref": "#/definitions/MyType", "description": "a ref"})),
        Just(json!({"$ref": "#/definitions/MyType", "title": "MyType"})),
        Just(json!({"$ref": "#/definitions/MyType", "title": "T", "description": "ref with both"})),
    ]
}

fn leaf_schema() -> impl Strategy<Value = Value> {
    prop_oneof![
        4 => primitive_schema(),
        1 => ref_schema(),
    ]
}

/// Full recursive property schema — all shapes, up to depth 3.
fn property_schema() -> impl Strategy<Value = Value> {
    leaf_schema().prop_recursive(
        3,  // max recursion depth
        48, // max total nodes across the tree
        5,  // max items per collection
        |inner| {
            prop_oneof![
                // --- anyOf variants ---
                // nullable (T | null) — the most common anyOf pattern
                inner.clone().prop_map(|a| json!({"anyOf": [a, {"type": "null"}]})),
                inner.clone().prop_map(|a| json!({"anyOf": [a, {"type": "null"}], "description": "nullable"})),
                // union of two types
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"anyOf": [a, b]})),
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"anyOf": [a, b], "description": "union"})),
                // IDA MCP pattern: anyOf [array-of-T, T] + description (the bug we fixed)
                inner.clone().prop_map(|t| json!({
                    "anyOf": [{"type": "array", "items": t}, {"type": "string"}],
                    "description": "one or many"
                })),
                // three-way anyOf
                (inner.clone(), inner.clone(), inner.clone())
                    .prop_map(|(a, b, c)| json!({"anyOf": [a, b, c]})),
                // anyOf containing $ref branches
                inner.clone().prop_map(|a| json!({"anyOf": [{"$ref": "#/definitions/T"}, a]})),
                // anyOf with title
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"anyOf": [a, b], "title": "Either"})),

                // --- oneOf variants ---
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"oneOf": [a, b]})),
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"oneOf": [a, b], "description": "exactly one"})),
                inner.clone().prop_map(|a| json!({"oneOf": [a, {"type": "null"}]})),
                // oneOf with $ref branch
                inner.clone().prop_map(|a| json!({"oneOf": [{"$ref": "#/definitions/T"}, a]})),

                // --- allOf variants ---
                inner.clone().prop_map(|a| json!({"allOf": [a]})),
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"allOf": [a, b]})),
                inner.clone().prop_map(|a| json!({"allOf": [a], "description": "refined"})),

                // --- not ---
                inner.clone().prop_map(|a| json!({"not": a})),

                // --- array variants ---
                inner.clone().prop_map(|items| json!({"type": "array", "items": items})),
                inner.clone().prop_map(|items| json!({"type": "array", "items": items, "description": "a list"})),
                inner.clone().prop_map(|items| json!({"type": "array", "items": items, "minItems": 1})),
                inner.clone().prop_map(|items| json!({"type": "array", "items": items, "minItems": 1, "maxItems": 10})),
                // tuple-style array (items is an array of schemas)
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"type": "array", "items": [a, b]})),
                // array with anyOf items
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| json!({"type": "array", "items": {"anyOf": [a, b]}})),
                // array of objects
                prop::collection::btree_map("[a-z]{2,6}", inner.clone(), 1..3)
                    .prop_map(|props| {
                        let m: serde_json::Map<_, _> = props.into_iter().collect();
                        json!({"type": "array", "items": {"type": "object", "properties": Value::Object(m)}})
                    }),

                // --- object variants ---
                // plain object with properties
                prop::collection::btree_map("[a-z]{2,6}", inner.clone(), 0..4)
                    .prop_map(|props| {
                        let m: serde_json::Map<_, _> = props.into_iter().collect();
                        json!({"type": "object", "properties": Value::Object(m)})
                    }),
                // object with required + additionalProperties: false
                prop::collection::btree_map("[a-z]{2,6}", inner.clone(), 1..4)
                    .prop_map(|props| {
                        let keys: Vec<String> = props.keys().take(1).cloned().collect();
                        let m: serde_json::Map<_, _> = props.into_iter().collect();
                        json!({
                            "type": "object",
                            "properties": Value::Object(m),
                            "required": keys,
                            "additionalProperties": false
                        })
                    }),
                // object with additionalProperties as a schema
                (
                    prop::collection::btree_map("[a-z]{2,6}", inner.clone(), 0..3),
                    inner.clone(),
                ).prop_map(|(props, additional)| {
                    let m: serde_json::Map<_, _> = props.into_iter().collect();
                    json!({
                        "type": "object",
                        "properties": Value::Object(m),
                        "additionalProperties": additional
                    })
                }),
                // object with patternProperties
                inner.clone().prop_map(|val_schema| json!({
                    "type": "object",
                    "patternProperties": {"^[a-z]+$": val_schema}
                })),
                // implied object — no "type" key, just properties (default-to-object path)
                prop::collection::btree_map("[a-z]{2,6}", inner.clone(), 1..4)
                    .prop_map(|props| {
                        let m: serde_json::Map<_, _> = props.into_iter().collect();
                        json!({"properties": Value::Object(m)})
                    }),
                // object with description
                prop::collection::btree_map("[a-z]{2,6}", inner.clone(), 1..3)
                    .prop_map(|props| {
                        let m: serde_json::Map<_, _> = props.into_iter().collect();
                        json!({
                            "type": "object",
                            "properties": Value::Object(m),
                            "description": "a nested object"
                        })
                    }),
            ]
        },
    )
}

/// Wraps a property schema in a complete tool definition.
fn make_tool(field_schema: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "test_tool",
            "description": "generated by proptest",
            "parameters": {
                "type": "object",
                "properties": {
                    "field": field_schema
                },
                "required": []
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Invariant helpers
// ---------------------------------------------------------------------------

/// True if the schema uses anyOf/oneOf/allOf as its primary type construct.
fn is_combinator(schema: &Value) -> bool {
    schema.get("anyOf").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("allOf").is_some()
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    /// Every schema that comes out of validate_tool_schemas must be accepted by
    /// the JSON Schema meta-schema validator — the same oracle a strict provider
    /// like Kimi uses internally.
    #[test]
    fn output_is_always_valid_json_schema(prop_schema in property_schema()) {
        let mut tools = vec![make_tool(prop_schema)];
        validate_tool_schemas(&mut tools);

        let parameters = &tools[0]["function"]["parameters"];
        jsonschema::meta::validate(parameters)
            .map_err(|e| TestCaseError::fail(format!(
                "validate_tool_schemas produced an invalid JSON Schema: {e}\nschema: {parameters}"
            )))?;
    }

    /// anyOf/oneOf/allOf properties must not have type/properties/required
    /// injected alongside them — that produces a contradictory schema.
    #[test]
    fn combinator_properties_are_not_corrupted(prop_schema in property_schema()) {
        let original_is_combinator = is_combinator(&prop_schema);
        let mut tools = vec![make_tool(prop_schema)];
        validate_tool_schemas(&mut tools);

        let field = &tools[0]["function"]["parameters"]["properties"]["field"];

        if original_is_combinator || is_combinator(field) {
            prop_assert!(
                field.get("type").is_none(),
                "combinator property got `type` injected: {field}"
            );
            prop_assert!(
                field.get("properties").is_none(),
                "combinator property got `properties` injected: {field}"
            );
            prop_assert!(
                field.get("required").is_none(),
                "combinator property got `required` injected: {field}"
            );
        }
    }

    /// $ref with sibling keywords must be wrapped in allOf (Kimi requirement),
    /// and the siblings must be preserved at the property level.
    #[test]
    fn ref_with_siblings_gets_wrapped_in_allof(
        prop_schema in ref_schema().prop_filter("must have siblings", |s| {
            s.as_object().map(|o| o.len() > 1).unwrap_or(false)
        })
    ) {
        let mut tools = vec![make_tool(prop_schema.clone())];
        validate_tool_schemas(&mut tools);

        let field = &tools[0]["function"]["parameters"]["properties"]["field"];
        prop_assert!(
            field.get("$ref").is_none(),
            "bare $ref should have been moved into allOf: {field}"
        );
        prop_assert!(
            field.get("allOf").is_some(),
            "$ref should be wrapped in allOf: {field}"
        );
        // siblings (description, title) should remain at the property level
        for (key, val) in prop_schema.as_object().unwrap() {
            if key != "$ref" {
                prop_assert_eq!(
                    field.get(key.as_str()),
                    Some(val),
                    "sibling key `{key}` was lost during $ref wrapping: {field}"
                );
            }
        }
    }
}
