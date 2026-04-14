use std::collections::HashMap;
use std::fs;
use pulldown_cmark::Parser;
use similar::TextDiff;

#[derive(Debug, Clone)]
pub struct SystemManifest {
    pub raw: String,
    pub sections: HashMap<String, String>,
}

impl SystemManifest {
    pub fn load(path: &str) -> std::io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        let sections = Self::parse_sections(&raw);
        Ok(Self { raw, sections })
    }

    pub fn load_from_string(raw: &str) -> Self {
        let sections = Self::parse_sections(raw);
        Self { raw: raw.to_string(), sections }
    }

    pub fn parse_sections(raw: &str) -> HashMap<String, String> {
        let mut sections = HashMap::new();
        let mut current_section = String::new();
        let mut current_content = String::new();

        for line in raw.lines() {
            if line.starts_with("## ") {
                // Save previous section
                if !current_section.is_empty() {
                    sections.insert(current_section.clone(), current_content.trim().to_string());
                }
                // Start new section
                current_section = line.trim_start_matches("## ").to_string();
                current_content.clear();
            } else {
                current_content.push_str(line);
                current_content.push('\n');
            }
        }
        // Save last section
        if !current_section.is_empty() {
            sections.insert(current_section, current_content.trim().to_string());
        }
        sections
    }

    /// Compute a unified diff between this manifest and another
    pub fn diff(&self, other: &SystemManifest) -> String {
        let diff = TextDiff::from_lines(&self.raw, &other.raw);

        diff.unified_diff()
            .context_radius(3)
            .to_string()
    }
}