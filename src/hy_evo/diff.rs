//! Genome Diffing and History (Task 37)
use crate::hy_evo::genome::WorkflowGenome;

pub fn diff(a: &WorkflowGenome, b: &WorkflowGenome) -> String {
    format!(
        "Genome Diff:\n  Nodes: {} → {}\n  Edges: {} → {}\n  Score: {} → {}\n  Version: {} → {}",
        a.nodes.len(),
        b.nodes.len(),
        a.edges.len(),
        b.edges.len(),
        a.score,
        b.score,
        a.version,
        b.version
    )
}
