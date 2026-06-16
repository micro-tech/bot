//! Final Answer handling — terminates the agent cleanly.

use log::info;

pub fn handle_final_answer(answer: &str) -> String {
    info!("Agent produced final answer: {}", answer);
    answer.to_string()
}
