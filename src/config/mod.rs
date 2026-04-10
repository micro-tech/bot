<content><![CDATA[
pub mod manifest;

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct OllamaConfig {
    pub name: String,
    pub url: String,
    pub model: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BotConfig {
    pub name: String
