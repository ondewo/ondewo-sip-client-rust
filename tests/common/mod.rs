// Copyright 2021-2026 ONDEWO GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The API surface as the GENERATED code and the PROTOS declare it, so that no test has to carry a
//! hand-written list or count of RPCs that goes stale with the next ondewo-sip-api release.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The committed tonic stubs of the `ondewo.sip` package.
const SIP_TONIC_STUBS: &str = include_str!("../../src/api/ondewo.sip.tonic.rs");

/// `SipGetSipStatus` -> `sip_get_sip_status`, the way prost names a generated method.
pub fn snake_case(rpc: &str) -> String {
    let mut snake: String = String::new();
    for (index, character) in rpc.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                snake.push('_');
            }
            snake.push(character.to_ascii_lowercase());
        } else {
            snake.push(character);
        }
    }
    snake
}

/// Every `service -> {rpc}` the generated SERVER routes, read from its
/// `"/ondewo.sip.<Service>/<Rpc>" => {` match arms.
pub fn generated_rpcs() -> BTreeMap<String, BTreeSet<String>> {
    let mut services: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in SIP_TONIC_STUBS.lines() {
        let Some(route) = line
            .trim()
            .strip_prefix("\"/ondewo.sip.")
            .and_then(|rest| rest.strip_suffix("\" => {"))
        else {
            continue;
        };
        let (service, rpc) = route.split_once('/').expect("a route is <Service>/<Rpc>");
        services
            .entry(service.to_string())
            .or_default()
            .insert(rpc.to_string());
    }
    assert!(
        !services.is_empty(),
        "no route found in src/api/ondewo.sip.tonic.rs"
    );
    services
}

/// The generated client methods of one service, in prost's snake_case.
pub fn generated_methods(service: &str) -> BTreeSet<String> {
    generated_rpcs()
        .remove(service)
        .unwrap_or_else(|| panic!("the generated code has no {service} service"))
        .iter()
        .map(|rpc| snake_case(rpc))
        .collect()
}

/// The `ondewo/sip` protos of the ondewo-sip-api submodule.
pub fn sip_proto_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ondewo-sip-api/ondewo/sip")
}

/// Every `service -> {rpc}` the protos declare (`service X {` ... `rpc Y (`).
pub fn proto_rpcs() -> BTreeMap<String, BTreeSet<String>> {
    let directory: PathBuf = sip_proto_dir();
    let entries = fs::read_dir(&directory).unwrap_or_else(|error| {
        panic!(
            "{}: {error} - check the API submodule out: git submodule update --init ondewo-sip-api",
            directory.display()
        )
    });
    let mut services: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in entries {
        let path: PathBuf = entry.expect("a directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("proto") {
            continue;
        }
        let text: String = fs::read_to_string(&path).expect("the proto is readable");
        // The service an rpc belongs to, and how deep inside its braces we are: an rpc may
        // carry its own `{ option ... }` block, so a `}` alone does not end the service.
        let mut current: Option<String> = None;
        let mut depth: usize = 0;
        for line in text.lines() {
            let line: &str = line.split("//").next().unwrap_or_default().trim();
            if current.is_none() {
                if let Some(rest) = line.strip_prefix("service ") {
                    let name: &str = rest
                        .split(|c: char| c == '{' || c.is_whitespace())
                        .next()
                        .unwrap();
                    current = Some(name.to_string());
                    services.entry(name.to_string()).or_default();
                    depth = 0;
                } else {
                    continue;
                }
            } else if let Some(rest) = line.strip_prefix("rpc ") {
                let name: &str = rest
                    .split(|c: char| c == '(' || c.is_whitespace())
                    .next()
                    .unwrap();
                let service: &String = current.as_ref().unwrap();
                services.get_mut(service).unwrap().insert(name.to_string());
            }
            depth += line.matches('{').count();
            depth -= line.matches('}').count().min(depth);
            if depth == 0 && line.contains('}') {
                current = None;
            }
        }
    }
    assert!(
        !services.is_empty(),
        "no service found under {}",
        directory.display()
    );
    services
}
