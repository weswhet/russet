//! Plist report structures matching the reference's receipt and summary keys.
use plist::{Dictionary, Value};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub failures: Vec<Dictionary>,
    pub summary_results: Dictionary,
}
impl Report {
    pub fn add_failure(
        &mut self,
        recipe: &str,
        identifier: Option<&str>,
        message: &str,
        traceback: &str,
    ) {
        let mut failure = Dictionary::from_iter([
            ("recipe", Value::String(recipe.into())),
            ("message", Value::String(message.into())),
            ("traceback", Value::String(traceback.into())),
        ]);
        if let Some(id) = identifier {
            failure.insert("recipe_id".into(), Value::String(id.into()));
        }
        self.failures.push(failure);
    }
    pub fn add_receipt(&mut self, receipt: &[Value]) -> Result<(), String> {
        for entry in receipt {
            let Some(output) = entry
                .as_dictionary()
                .and_then(|d| d.get("Output"))
                .and_then(Value::as_dictionary)
            else {
                continue;
            };
            for (key, value) in output {
                if !key.ends_with("_summary_result") {
                    continue;
                }
                let result = value
                    .as_dictionary()
                    .ok_or("Summary result must be a dictionary")?;
                let Some(data) = result
                    .get("data")
                    .and_then(Value::as_dictionary)
                    .filter(|d| !d.is_empty())
                else {
                    continue;
                };
                if !self.summary_results.contains_key(key) {
                    self.summary_results.insert(
                        key.clone(),
                        Value::Dictionary(Dictionary::from_iter([
                            (
                                "summary_text",
                                result
                                    .get("summary_text")
                                    .cloned()
                                    .unwrap_or(Value::String(String::new())),
                            ),
                            (
                                "header",
                                result
                                    .get("report_fields")
                                    .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        Value::Array(
                                            data.keys().cloned().map(Value::String).collect(),
                                        )
                                    }),
                            ),
                            ("data_rows", Value::Array(Vec::new())),
                        ])),
                    );
                }
                self.summary_results
                    .get_mut(key)
                    .and_then(Value::as_dictionary_mut)
                    .and_then(|d| d.get_mut("data_rows"))
                    .and_then(Value::as_array_mut)
                    .ok_or("Invalid accumulated summary")?
                    .push(Value::Dictionary(data.clone()));
            }
        }
        Ok(())
    }
    pub fn to_plist(&self) -> Value {
        Value::Dictionary(Dictionary::from_iter([
            (
                "failures",
                Value::Array(
                    self.failures
                        .iter()
                        .cloned()
                        .map(Value::Dictionary)
                        .collect(),
                ),
            ),
            (
                "summary_results",
                Value::Dictionary(self.summary_results.clone()),
            ),
        ]))
    }
    pub fn write(&self, path: &Path) -> Result<(), String> {
        self.to_plist().to_file_xml(path).map_err(|e| e.to_string())
    }
}
pub fn write_receipt(receipt: &[Value], path: &Path) -> Result<(), String> {
    Value::Array(receipt.to_vec())
        .to_file_xml(path)
        .map_err(|e| e.to_string())
}
pub fn write_run_results(receipts: &[Vec<Value>], path: &Path) -> Result<(), String> {
    Value::Array(receipts.iter().cloned().map(Value::Array).collect())
        .to_file_xml(path)
        .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregates_summary_rows_and_failure_shape() {
        let summary = Value::Dictionary(Dictionary::from_iter([
            ("summary_text", Value::String("Created files".into())),
            (
                "data",
                Value::Dictionary(Dictionary::from_iter([(
                    "path",
                    Value::String("sample".into()),
                )])),
            ),
        ]));
        let receipt = vec![Value::Dictionary(Dictionary::from_iter([(
            "Output",
            Value::Dictionary(Dictionary::from_iter([("test_summary_result", summary)])),
        )]))];
        let mut report = Report::default();
        report.add_receipt(&receipt).unwrap();
        report.add_receipt(&receipt).unwrap();
        assert_eq!(
            report.summary_results["test_summary_result"]
                .as_dictionary()
                .unwrap()["data_rows"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        report.add_failure("test.recipe", Some("org.test"), "failed", "");
        assert_eq!(
            report.failures[0]["recipe_id"].as_string(),
            Some("org.test")
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("report.plist");
        report.write(&path).unwrap();
        assert_eq!(Value::from_file(path).unwrap(), report.to_plist());
    }
}
