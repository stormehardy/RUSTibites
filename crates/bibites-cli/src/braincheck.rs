//! `rustibites braincheck` — emit the Rust brain evaluator's outputs for the
//! independent Python reference harness in `tools/braincheck/`.
//!
//! The fixture/output contract is documented in `tools/braincheck/README.md`.
//! This is validation tooling: it reads a fixture (brain topology + initial
//! runtime state + input sequences), evaluates each sequence with
//! [`bibites_core::brain::Brain::step`], and writes the resulting per-tick node
//! state so `run_check.py` can compare it bitwise against the independent Python
//! implementation.

use std::path::Path;

use bibites_core::brain::{Brain, BrainInputs, Input, NodeRecord, NodeType, Output, SynapseRecord};
use serde::{Deserialize, Serialize};

const N_INPUTS: usize = 33;
const N_OUTPUTS: usize = 15;

#[derive(Debug, Deserialize)]
struct Fixture {
    period: f32,
    #[serde(default = "default_true")]
    herding_enabled: bool,
    nodes: Vec<FixtureNode>,
    synapses: Vec<FixtureSynapse>,
    input_sequences: Vec<Vec<Vec<f32>>>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct FixtureNode {
    #[serde(rename = "Type")]
    node_type: u8,
    #[serde(rename = "Index")]
    index: u32,
    #[serde(rename = "Inov", default)]
    innovation: i64,
    #[serde(rename = "baseActivation", default)]
    base_activation: f32,
    #[serde(rename = "Value", default)]
    value: f32,
    #[serde(rename = "LastInput", default)]
    last_input: f32,
    #[serde(rename = "LastOutput", default)]
    last_output: f32,
}

#[derive(Debug, Deserialize)]
struct FixtureSynapse {
    #[serde(rename = "Inov", default)]
    innovation: i64,
    #[serde(rename = "NodeIn")]
    node_in: u32,
    #[serde(rename = "NodeOut")]
    node_out: u32,
    #[serde(rename = "Weight", default)]
    weight: f32,
    #[serde(rename = "En", default)]
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct BrainCheckOutput {
    period: f32,
    herding_enabled: bool,
    /// Diagnostic only; ignored by the comparator.
    libm: String,
    tick_count: usize,
    ticks: Vec<TickOutput>,
}

#[derive(Debug, Serialize)]
struct TickOutput {
    sequence: usize,
    tick: usize,
    inputs: Vec<f32>,
    outputs: Vec<f32>,
    nodes: Vec<NodeOutput>,
}

#[derive(Debug, Serialize)]
struct NodeOutput {
    #[serde(rename = "Index")]
    index: u32,
    #[serde(rename = "Type")]
    node_type: u8,
    #[serde(rename = "Value")]
    value: f32,
    #[serde(rename = "LastInput")]
    last_input: f32,
    #[serde(rename = "LastOutput")]
    last_output: f32,
}

/// Runs the brain check for `fixture_path`; writes JSON to `out` or stdout.
pub fn run(fixture_path: &Path, out: Option<&Path>) -> Result<(), String> {
    let text = std::fs::read_to_string(fixture_path)
        .map_err(|error| format!("could not read {}: {error}", fixture_path.display()))?;
    let fixture: Fixture = serde_json::from_str(&text)
        .map_err(|error| format!("invalid fixture {}: {error}", fixture_path.display()))?;

    if fixture.input_sequences.is_empty() {
        return Err("fixture has no input_sequences".into());
    }
    for (sequence_index, sequence) in fixture.input_sequences.iter().enumerate() {
        for (tick, inputs) in sequence.iter().enumerate() {
            if inputs.len() != N_INPUTS {
                return Err(format!(
                    "sequence {sequence_index} tick {tick}: expected {N_INPUTS} inputs, got {}",
                    inputs.len()
                ));
            }
        }
    }

    let node_records = fixture
        .nodes
        .iter()
        .map(|node| {
            let node_type = NodeType::from_u8(node.node_type).ok_or_else(|| {
                format!("node {}: unknown NodeType {}", node.index, node.node_type)
            })?;
            Ok(NodeRecord {
                node_type,
                index: node.index,
                innovation: node.innovation,
                desc: None,
                base_activation: node.base_activation,
                value: node.value,
                last_input: node.last_input,
                last_output: node.last_output,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let synapse_records: Vec<SynapseRecord> = fixture
        .synapses
        .iter()
        .map(|synapse| SynapseRecord {
            innovation: synapse.innovation,
            node_in: synapse.node_in,
            node_out: synapse.node_out,
            weight: synapse.weight,
            enabled: synapse.enabled,
        })
        .collect();

    let mut tick_count = 0usize;
    let mut ticks = Vec::new();
    for (sequence_index, sequence) in fixture.input_sequences.iter().enumerate() {
        // Each sequence is an independent run from the fixture's initial state.
        let mut brain = Brain::from_records(&node_records, &synapse_records)
            .map_err(|error| format!("could not build brain: {error}"))?;
        brain.set_herding_enabled(fixture.herding_enabled);
        brain.rebuild();

        for (tick, values) in sequence.iter().enumerate() {
            let mut inputs = BrainInputs::new();
            for (index, value) in values.iter().enumerate() {
                let input = Input::from_index(index)
                    .ok_or_else(|| format!("no Input for index {index}"))?;
                inputs.set(input, *value);
            }
            let _ = brain.step(&inputs, fixture.period);

            let mut outputs = vec![0.0f32; N_OUTPUTS];
            for (index, slot) in outputs.iter_mut().enumerate() {
                let output = Output::from_index(index)
                    .ok_or_else(|| format!("no Output for index {index}"))?;
                *slot = brain.output_value(output);
            }

            let mut nodes = Vec::with_capacity(brain.node_count());
            for index in 0..brain.node_count() {
                let state = brain
                    .node_state(index)
                    .ok_or_else(|| format!("missing node state at index {index}"))?;
                let node_type = brain
                    .node_type(index)
                    .ok_or_else(|| format!("missing node type at index {index}"))?;
                nodes.push(NodeOutput {
                    index: index as u32,
                    node_type: node_type as u8,
                    value: state.value,
                    last_input: state.last_input,
                    last_output: state.last_output,
                });
            }

            ticks.push(TickOutput {
                sequence: sequence_index,
                tick,
                inputs: values.clone(),
                outputs,
                nodes,
            });
            tick_count += 1;
        }
    }

    let output = BrainCheckOutput {
        period: fixture.period,
        herding_enabled: fixture.herding_enabled,
        libm: "rust".to_owned(),
        tick_count,
        ticks,
    };
    let json = serde_json::to_string_pretty(&output)
        .map_err(|error| format!("could not serialize braincheck output: {error}"))?;

    match out {
        Some(path) => std::fs::write(path, json)
            .map_err(|error| format!("could not write {}: {error}", path.display())),
        None => {
            println!("{json}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_contract_is_parsed() {
        let json = r#"{
            "period": 0.06666667,
            "nodes": [
                {"Type": 0, "Index": 0, "Inov": 0, "baseActivation": 0.0,
                 "Value": 0.0, "LastInput": 0.0, "LastOutput": 0.0}
            ],
            "synapses": [],
            "input_sequences": [[[0.0]]]
        }"#;
        let fixture: Fixture = serde_json::from_str(json).expect("fixture parses");
        assert!(fixture.herding_enabled);
        assert_eq!(fixture.nodes.len(), 1);
    }

    #[test]
    fn output_serializes_pascal_case_node_keys() {
        let node = NodeOutput {
            index: 3,
            node_type: 1,
            value: 0.5,
            last_input: 0.25,
            last_output: 0.5,
        };
        let json = serde_json::to_string(&node).expect("serializes");
        assert!(json.contains("\"Index\":3"));
        assert!(json.contains("\"Type\":1"));
        assert!(json.contains("\"LastInput\":0.25"));
    }
}
