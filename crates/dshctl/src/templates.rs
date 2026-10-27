// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runtime files embedded in the binary so a standalone `dshctl` install does
//! not need a source checkout (the prebuilt image path uses these directly).

pub const COMPOSE_YAML: &str = include_str!("../../../compose.yaml");
pub const ENV_EXAMPLE: &str = include_str!("../../../.env.example");
