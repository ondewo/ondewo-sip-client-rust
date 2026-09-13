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

//! Wire-level tests for the GENERATED prost messages under `src/api`.
//!
//! These are the cases that catch a broken generator: a dropped field, a shifted tag number, an
//! enum whose discriminants moved, a map that lost its entries. They are pure encode/decode - no
//! runtime, no socket. The gRPC plumbing is covered by `tests/generated_grpc.rs`.
//!
//! Two cases of the NLU client's equivalent suite have no counterpart here, deliberately:
//!
//! * there is no explicit-presence case, because `ondewo-sip-api` declares no scalar proto3
//!   `optional` field at all - `SipStatus.timestamp` and the other `Option<T>` fields are
//!   message-typed, which prost models as `Option` regardless of presence and which would
//!   therefore prove nothing about presence handling;
//! * there is no cross-package case, because the SIP API compiles to exactly one package,
//!   `ondewo.sip` - it vendors no `google.*` proto and no second ONDEWO package.

use std::collections::HashMap;

use ondewo_sip_client::api::ondewo::sip;
use prost::Message;
use prost_types::Timestamp;

/// A fully populated [`sip::SipStatus`] - scalar, map, message and enum fields at once.
fn sample_status() -> sip::SipStatus {
    let mut headers = HashMap::new();
    headers.insert("X-Ondewo-Call".to_string(), "outbound".to_string());
    headers.insert("X-Ondewo-Agent".to_string(), "agent-7".to_string());

    sip::SipStatus {
        account_name: "sip-user-1@mydomain.com".to_string(),
        timestamp: Some(Timestamp {
            seconds: 1_700_000_000,
            nanos: 123,
        }),
        status_type: sip::sip_status::StatusType::OutgoingCallConnected as i32,
        callee_id: "sip-user-2@mydomain.com".to_string(),
        transfer_call_id: "sip-user-3@mydomain.com".to_string(),
        headers,
        description: "the outbound call is connected".to_string(),
        exception_name: String::new(),
        exception_traceback: String::new(),
        nlu_session_name: "projects/p/agent/sessions/s".to_string(),
    }
}

#[test]
fn sip_status_survives_a_serialize_parse_round_trip() {
    let original = sample_status();

    let bytes = original.encode_to_vec();
    assert!(
        !bytes.is_empty(),
        "a populated SipStatus must not encode to zero bytes"
    );
    assert_eq!(
        bytes.len(),
        original.encoded_len(),
        "encoded_len must agree with the bytes actually written"
    );

    let parsed =
        sip::SipStatus::decode(bytes.as_slice()).expect("re-parsing our own bytes must work");
    assert_eq!(parsed, original);

    // Spot-check the individual fields too: a PartialEq on two identically broken values would
    // still pass above.
    assert_eq!(parsed.account_name, "sip-user-1@mydomain.com");
    assert_eq!(parsed.callee_id, "sip-user-2@mydomain.com");
    assert_eq!(parsed.timestamp.unwrap().nanos, 123);
    assert_eq!(
        parsed.status_type,
        sip::sip_status::StatusType::OutgoingCallConnected as i32
    );
    assert_eq!(parsed.headers["X-Ondewo-Call"], "outbound");
    assert_eq!(parsed.headers["X-Ondewo-Agent"], "agent-7");
    assert_eq!(parsed.nlu_session_name, "projects/p/agent/sessions/s");
}

#[test]
fn a_default_sip_status_round_trips_to_zero_bytes() {
    let empty = sip::SipStatus::default();

    assert_eq!(empty.account_name, "");
    assert_eq!(empty.timestamp, None);
    assert_eq!(empty.status_type, 0);
    assert!(empty.headers.is_empty());

    let bytes = empty.encode_to_vec();
    assert!(
        bytes.is_empty(),
        "proto3 must not put unset fields on the wire, got {bytes:?}"
    );
    assert_eq!(sip::SipStatus::decode(bytes.as_slice()).unwrap(), empty);
}

/// Decoding tolerates fields it does not know: an unknown tag is skipped, not an error.
#[test]
fn decoding_skips_an_unknown_field() {
    let mut bytes = sip::SipStartSessionRequest {
        account_name: "sip-user-1@mydomain.com".to_string(),
        auto_answer_interval: 3,
    }
    .encode_to_vec();
    // tag 999, wire type 0 (varint), value 1
    bytes.extend_from_slice(&[0xB8, 0x3E, 0x01]);

    let parsed = sip::SipStartSessionRequest::decode(bytes.as_slice())
        .expect("an unknown field must be skipped, not rejected");
    assert_eq!(parsed.account_name, "sip-user-1@mydomain.com");
    assert_eq!(parsed.auto_answer_interval, 3);
}

#[test]
fn decoding_rejects_a_truncated_message() {
    // Four plain string fields, so the last byte on the wire is the last byte of the last string
    // and dropping it leaves a length prefix that cannot be satisfied.
    let bytes = sip::SipRegisterAccountRequest {
        account_name: "sip-user-1@mydomain.com".to_string(),
        password: "hunter2".to_string(),
        auth_username: "sip-user-1".to_string(),
        outbound_proxy: "my.outbound.proxy.com".to_string(),
    }
    .encode_to_vec();
    let truncated = &bytes[..bytes.len() - 1];

    assert!(
        sip::SipRegisterAccountRequest::decode(truncated).is_err(),
        "a truncated message must not decode silently"
    );
}

/// The zero value of an enum is the one a default-constructed message carries, so it must be the
/// variant the proto declares as `= 0`. For `SipStatus.StatusType` that is `NO_SESSION`, not an
/// `…_UNSPECIFIED` variant - the SIP API names its zero state.
#[test]
fn the_enum_zero_value_is_the_variant_the_proto_declares_as_zero() {
    use sip::sip_status::StatusType;

    assert_eq!(StatusType::NoSession as i32, 0);
    assert_eq!(StatusType::try_from(0), Ok(StatusType::NoSession));
    assert_eq!(
        sip::SipStatus::default().status_type,
        StatusType::NoSession as i32,
        "a default message must carry the enum's zero value"
    );

    assert_eq!(StatusType::NoSession.as_str_name(), "NO_SESSION");
    assert_eq!(
        StatusType::from_str_name("NO_SESSION"),
        Some(StatusType::NoSession)
    );
    assert_eq!(StatusType::from_str_name("NOT_A_VARIANT"), None);
    assert!(
        StatusType::try_from(9_999).is_err(),
        "an out-of-range discriminant must not map to a variant"
    );
}

/// A non-zero enum value has to travel as its discriminant, not as the zero value.
#[test]
fn a_non_zero_enum_value_round_trips() {
    use sip::sip_status::StatusType;

    let status = sip::SipStatus {
        account_name: "sip-user-1@mydomain.com".to_string(),
        status_type: StatusType::TransferCallFailed as i32,
        ..Default::default()
    };

    let parsed = sip::SipStatus::decode(status.encode_to_vec().as_slice()).unwrap();
    assert_eq!(parsed, status);
    assert_eq!(
        StatusType::try_from(parsed.status_type),
        Ok(StatusType::TransferCallFailed)
    );
    assert_eq!(
        StatusType::TransferCallFailed.as_str_name(),
        "TRANSFER_CALL_FAILED"
    );
}

/// Repeated and nested message fields have to nest, not flatten.
#[test]
fn a_nested_and_repeated_message_round_trips() {
    let response = sip::SipStatusHistoryResponse {
        status_history: vec![
            sample_status(),
            sip::SipStatus {
                account_name: "second".to_string(),
                status_type: sip::sip_status::StatusType::SessionEnded as i32,
                ..Default::default()
            },
        ],
    };

    let parsed =
        sip::SipStatusHistoryResponse::decode(response.encode_to_vec().as_slice()).unwrap();
    assert_eq!(parsed, response);
    assert_eq!(parsed.status_history.len(), 2);
    assert_eq!(parsed.status_history[1].account_name, "second");
    assert_eq!(
        parsed.status_history[0].headers.len(),
        2,
        "the nested map must survive being nested"
    );
}

/// A repeated `bytes` field is a `Vec<Vec<u8>>`, not a flattened blob - the wav payloads have to
/// come back as the same number of files, each byte-identical.
#[test]
fn a_repeated_bytes_field_keeps_its_element_boundaries() {
    let request = sip::SipPlayWavFilesRequest {
        wav_files: vec![vec![0x52, 0x49, 0x46, 0x46], vec![], vec![0xFF, 0x00, 0xFF]],
    };

    let parsed = sip::SipPlayWavFilesRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    assert_eq!(parsed, request);
    assert_eq!(parsed.wav_files.len(), 3);
    assert_eq!(parsed.wav_files[1], Vec::<u8>::new());
    assert_eq!(parsed.wav_files[2], vec![0xFF, 0x00, 0xFF]);
}
