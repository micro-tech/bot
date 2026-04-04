use std::fs;

#[derive(Debug)]
pub struct SystemManifest {
    pub raw: String,
}

impl SystemManifest {
    pub fn load(path: &str) -> std::io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        Ok(Self { raw })
    }
}