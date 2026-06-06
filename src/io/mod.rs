// IO Module for Agent OS
// This mod.rs file declares the sub-modules for the IO component.

pub mod io;
#[cfg(unix)]
pub mod unix_socket;

// Including sub-modules with test files to run their tests
pub mod ollama;
pub mod web_server;
pub mod terminal;
pub mod llm_gemini;

// Test modules are typically not declared in mod.rs for library use,
// but for clarity, they are included in the project structure.
// Tests are run via cargo test and are defined in *_test.rs files.
