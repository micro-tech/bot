// IO Module for Helix
// This mod.rs file declares the sub-modules for the IO component.

pub mod io;
#[cfg(unix)]
pub mod unix_socket;

// Including sub-modules with test files to run their tests
pub mod browser;
pub mod llm_gemini;
pub mod ollama;
pub mod retry; // tasks 372-374 (H3/H4/H5): shared retry_with_backoff helper
pub mod terminal;
pub mod web_server; // tasks 350+: headless Firefox over WebDriver BiDi

// Test modules are typically not declared in mod.rs for library use,
// but for clarity, they are included in the project structure.
// Tests are run via cargo test and are defined in *_test.rs files.
