// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runtime files embedded in the binary so a standalone `dshctl` install does
//! not need a source checkout (the prebuilt image path uses these directly).

pub const COMPOSE_YAML: &str = include_str!("../../../compose.yaml");
pub const COMPOSE_BUILD_YAML: &str = include_str!("../../../compose.build.yaml");
pub const DOCKERFILE: &str = include_str!("../../../Dockerfile");
pub const ENV_EXAMPLE: &str = include_str!("../../../.env.example");
pub const DSH_HTTP_COMPAT_JS: &str = include_str!("../../../assets/dsh-http-compat.js");
