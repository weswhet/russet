//! The original frozen API, explicitly promoted compiled recipe processors,
//! and the processors Russet adds.
use serde_json::Value;
use std::sync::OnceLock;

pub fn community_contract() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../compatibility/community-processors.json"
        ))
        .expect("valid promoted processor contract")
    })
}

/// The processors Russet adds to AutoPkg's, such as `RussetURLDownloader`.
pub fn russet_contract() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../compatibility/russet-processors.json"
        ))
        .expect("valid Russet processor contract")
    })
}

pub fn canonical_name(name: &str) -> &str {
    community_contract()["aliases"]
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or(name)
}

pub fn community_source(name: &str) -> Option<&'static Value> {
    community_contract()["sources"].get(canonical_name(name))
}

pub fn contract() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut merged: Value =
            serde_json::from_str(include_str!("../../../../compatibility/reference.json"))
                .expect("valid frozen processor contract");
        let promoted = community_contract();
        let processors = merged["processors"].as_object_mut().unwrap();
        for (name, manifest) in promoted["processors"].as_object().unwrap() {
            assert!(processors.insert(name.clone(), manifest.clone()).is_none());
        }
        for (alias, canonical) in promoted["aliases"].as_object().unwrap() {
            let manifest = processors[canonical.as_str().unwrap()].clone();
            assert!(processors.insert(alias.clone(), manifest).is_none());
        }
        for (name, manifest) in russet_contract()["processors"].as_object().unwrap() {
            assert!(processors.insert(name.clone(), manifest.clone()).is_none());
        }
        merged
    })
}

pub fn processor_order() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut merged: Value = serde_json::from_str(include_str!(
            "../../../../compatibility/cli-processor-order.json"
        ))
        .expect("valid frozen processor order");
        let promoted = community_contract();
        let ordering = merged.as_object_mut().unwrap();
        for (name, fields) in promoted["ordering"].as_object().unwrap() {
            assert!(ordering.insert(name.clone(), fields.clone()).is_none());
        }
        for (alias, canonical) in promoted["aliases"].as_object().unwrap() {
            let fields = ordering[canonical.as_str().unwrap()].clone();
            assert!(ordering.insert(alias.clone(), fields).is_none());
        }
        for (name, fields) in russet_contract()["ordering"].as_object().unwrap() {
            assert!(ordering.insert(name.clone(), fields.clone()).is_none());
        }
        merged
    })
}
