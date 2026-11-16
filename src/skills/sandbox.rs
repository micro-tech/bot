//! Skill Sandboxing (WASM) stub (Task 40)
//!
//! NOT a real sandbox: `run_in_sandbox` just formats a string — no WASM,
//! no isolation, no resource limits. Nothing in Helix executes untrusted
//! skill code through this function. A full sandbox IS a prerequisite for
//! any self-deployment (task 366) and is a separate follow-up workstream
//! (task 368). Do not claim a sandbox exists.
pub fn run_in_sandbox(code: &str) -> Result<String, String> {
    // Placeholder — real WASM execution would go here
    Ok(format!("Executed in sandbox: {}", code))
}
