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

//! The committed stubs match the protos they were generated from.
//!
//! Both sides are DERIVED - the services and rpcs the `ondewo-sip-api` submodule declares, and
//! the routes the generated tonic servers answer - so this needs no update when the API grows,
//! and fails when the stubs were not regenerated after the API pin moved, or when the generator
//! dropped or renamed an rpc. The submodule must be checked out (CI does that).

mod common;

use std::collections::{BTreeMap, BTreeSet};

#[test]
fn the_generated_stubs_route_exactly_the_rpcs_the_protos_declare() {
    let declared: BTreeMap<String, BTreeSet<String>> = common::proto_rpcs();
    let generated: BTreeMap<String, BTreeSet<String>> = common::generated_rpcs();
    assert_eq!(generated, declared);
}

/// "Must contain" pins only: the services the hand-written tests drive by name.
#[test]
fn the_services_the_tests_drive_are_generated() {
    let generated: BTreeMap<String, BTreeSet<String>> = common::generated_rpcs();
    for service in ["Sip"] {
        assert!(
            generated.get(service).is_some_and(|rpcs| !rpcs.is_empty()),
            "{service}"
        );
    }
}

#[test]
fn rpc_names_become_prost_method_names() {
    assert_eq!(common::snake_case("SipGetSipStatus"), "sip_get_sip_status");
    assert_eq!(common::snake_case("SipUnMute"), "sip_un_mute");
}
