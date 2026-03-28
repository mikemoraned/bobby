use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ExpectedExamples {
    pub example: Vec<Example>,
}

#[derive(Debug, Deserialize)]
pub struct Example {
    pub path: String,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub rejected: Option<Vec<String>>,
    #[serde(default)]
    pub exemplar: bool,
}

pub fn load_examples(path: &Path) -> Result<Vec<Example>, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let parsed: ExpectedExamples = toml::from_str(&content)?;
    Ok(parsed.example)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_expected_toml() {
        let examples = load_examples(Path::new("../examples/expected.toml"))
            .expect("should load expected.toml");
        assert!(!examples.is_empty());
        assert!(examples.iter().any(|e| e.exemplar));
        assert!(examples.iter().any(|e| !e.exemplar));
    }
}
