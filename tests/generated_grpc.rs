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

//! End-to-end tests for the GENERATED tonic service stubs.
//!
//! The generated `SipServer` is served over a loopback socket and driven by the generated
//! `SipClient`, so a request really is encoded, routed by its `/ondewo.sip.Sip/<Method>` path,
//! decoded, answered and decoded again. That is what catches a service the generator wired to the
//! wrong path, a codec mismatch, or a method that silently went missing.
//!
//! Every RPC of `ondewo.sip.Sip` is unary except `SipStreamCallAudio`, a bidirectional stream;
//! it has its own case below that drives both directions of the stream over the socket.
//!
//! No network beyond `127.0.0.1` and no ONDEWO server is involved.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ondewo_sip_client::api::ondewo::sip;
use ondewo_sip_client::api::ondewo::sip::sip_client::SipClient;
use ondewo_sip_client::api::ondewo::sip::sip_server::{Sip, SipServer};
use ondewo_sip_client::auth::{
    BearerTokenInterceptor, AUTHORIZATION_METADATA_KEY, CAI_TOKEN_METADATA_KEY,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tokio_stream::{Stream, StreamExt};
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Code, Request, Response, Status, Streaming};

use sip::sip_call_audio_request::Request as AudioRequest;
use sip::sip_call_audio_response::Response as AudioResponse;
use sip::sip_status::StatusType;

/// The gRPC metadatum that scopes a request to one call (`SipStatus.call_id`).
const EXPECTED_CALL_ID_METADATA_KEY: &str = "x-ondewo-expected-call-id";

/// The server half of `SipStreamCallAudio`, as the generated `Sip` trait asks for it.
type CallAudioStream =
    Pin<Box<dyn Stream<Item = Result<sip::SipCallAudioResponse, Status>> + Send + 'static>>;

/// The callee `sip_start_call` answers with `not_found` for, so the error path is exercised too.
const UNKNOWN_CALLEE: &str = "does-not-exist@mydomain.com";

/// Metadata the fake server captured from the last request it handled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SeenMetadata {
    authorization: Option<String>,
    cai_token: Option<String>,
}

/// A minimal in-process implementation of the generated `Sip` service.
#[derive(Clone, Default)]
struct FakeSip {
    seen: Arc<Mutex<SeenMetadata>>,
}

impl FakeSip {
    fn record<T>(&self, request: &Request<T>) {
        let read = |key: &str| {
            request
                .metadata()
                .get(key)
                .map(|value| value.to_str().unwrap().to_string())
        };
        *self.seen.lock().unwrap() = SeenMetadata {
            authorization: read(AUTHORIZATION_METADATA_KEY),
            cai_token: read(CAI_TOKEN_METADATA_KEY),
        };
    }

    fn seen(&self) -> SeenMetadata {
        self.seen.lock().unwrap().clone()
    }
}

/// A `SipStatus` carrying just a status type, as most of the RPCs below answer with.
fn status_of(status_type: StatusType) -> sip::SipStatus {
    sip::SipStatus {
        account_name: "sip-user-1@mydomain.com".to_string(),
        status_type: status_type as i32,
        ..Default::default()
    }
}

#[tonic::async_trait]
impl Sip for FakeSip {
    async fn sip_start_session(
        &self,
        request: Request<sip::SipStartSessionRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(sip::SipStatus {
            account_name: request.into_inner().account_name,
            status_type: StatusType::SessionStarted as i32,
            ..Default::default()
        }))
    }

    async fn sip_end_session(
        &self,
        request: Request<()>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(status_of(StatusType::SessionEnded)))
    }

    /// Echoes the callee and the headers back, so the round trip proves a map field survives a
    /// real gRPC hop, and answers `not_found` for [`UNKNOWN_CALLEE`].
    async fn sip_start_call(
        &self,
        request: Request<sip::SipStartCallRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        let request = request.into_inner();
        if request.callee_id == UNKNOWN_CALLEE {
            return Err(Status::not_found(format!(
                "no sip account named {}",
                request.callee_id
            )));
        }
        Ok(Response::new(sip::SipStatus {
            account_name: "sip-user-1@mydomain.com".to_string(),
            status_type: StatusType::OutgoingCallInitiated as i32,
            callee_id: request.callee_id,
            headers: request.headers,
            ..Default::default()
        }))
    }

    async fn sip_end_call(
        &self,
        request: Request<sip::SipEndCallRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(status_of(
            if request.into_inner().hard_hangup {
                StatusType::HardHangupInitiated
            } else {
                StatusType::SoftHangupInitiated
            },
        )))
    }

    async fn sip_transfer_call(
        &self,
        request: Request<sip::SipTransferCallRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        let request = request.into_inner();
        Ok(Response::new(sip::SipStatus {
            account_name: "sip-user-1@mydomain.com".to_string(),
            status_type: StatusType::TransferCallInitiated as i32,
            transfer_call_id: request.transfer_id,
            headers: request.headers,
            ..Default::default()
        }))
    }

    async fn sip_register_account(
        &self,
        request: Request<sip::SipRegisterAccountRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(sip::SipStatus {
            account_name: request.into_inner().account_name,
            status_type: StatusType::Registered as i32,
            ..Default::default()
        }))
    }

    async fn sip_get_sip_status(
        &self,
        request: Request<()>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(status_of(StatusType::Ready)))
    }

    async fn sip_get_sip_status_history(
        &self,
        request: Request<()>,
    ) -> Result<Response<sip::SipStatusHistoryResponse>, Status> {
        self.record(&request);
        Ok(Response::new(sip::SipStatusHistoryResponse {
            status_history: vec![
                status_of(StatusType::SessionStarted),
                status_of(StatusType::Ready),
            ],
        }))
    }

    async fn sip_play_wav_files(
        &self,
        request: Request<sip::SipPlayWavFilesRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        let played = request.into_inner().wav_files.len();
        Ok(Response::new(sip::SipStatus {
            account_name: "sip-user-1@mydomain.com".to_string(),
            status_type: StatusType::MicrophoneWavFilesPlayed as i32,
            description: format!("played {played} wav files"),
            ..Default::default()
        }))
    }

    async fn sip_mute(&self, request: Request<()>) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(status_of(StatusType::MicrophoneMuted)))
    }

    async fn sip_un_mute(&self, request: Request<()>) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        Ok(Response::new(status_of(StatusType::MicrophoneUnmuted)))
    }

    /// Carries the reported detection result into the status, as the real server does.
    async fn sip_report_answering_machine_detected(
        &self,
        request: Request<sip::SipReportAnsweringMachineDetectedRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        let amd_result = request.into_inner().amd_result;
        Ok(Response::new(sip::SipStatus {
            status_type: StatusType::OutgoingCallAnsweringMachineDetected as i32,
            call_id: amd_result
                .as_ref()
                .map(|result| result.call_id.clone())
                .unwrap_or_default(),
            amd_result,
            ..status_of(StatusType::OutgoingCallAnsweringMachineDetected)
        }))
    }

    /// Requires the call-scoping metadatum, as the real server does, and reports the media
    /// control it was asked for in `bot_muted` / `listening_paused`.
    async fn sip_set_call_media_control(
        &self,
        request: Request<sip::SipSetCallMediaControlRequest>,
    ) -> Result<Response<sip::SipStatus>, Status> {
        self.record(&request);
        let call_id = request
            .metadata()
            .get(EXPECTED_CALL_ID_METADATA_KEY)
            .map(|value| value.to_str().unwrap().to_string())
            .ok_or_else(|| Status::failed_precondition("CallScopeMismatch"))?;
        let request = request.into_inner();
        Ok(Response::new(sip::SipStatus {
            call_id,
            bot_muted: request.bot_voice == sip::MediaControlSetting::Off as i32,
            listening_paused: request.bot_listening == sip::MediaControlSetting::Off as i32,
            ..status_of(StatusType::OutgoingCallConnected)
        }))
    }

    type SipStreamCallAudioStream = CallAudioStream;

    /// Answers the opening config with `started`, echoes every audio frame back and closes with
    /// `ended(CLIENT_CLOSED)` once the client half-closes its side of the stream.
    async fn sip_stream_call_audio(
        &self,
        request: Request<Streaming<sip::SipCallAudioRequest>>,
    ) -> Result<Response<Self::SipStreamCallAudioStream>, Status> {
        self.record(&request);
        let mut inbound = request.into_inner();
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let respond = |response| {
                Ok(sip::SipCallAudioResponse {
                    response: Some(response),
                })
            };
            while let Some(message) = inbound.next().await {
                let response = match message {
                    Ok(sip::SipCallAudioRequest {
                        request: Some(AudioRequest::Config(config)),
                    }) => respond(AudioResponse::Started(sip::SipCallAudioStarted {
                        stream_id: config.stream_id,
                        sample_rate_hz: config.sample_rate_hz,
                        frame_ms: config.frame_ms,
                        mode: config.mode,
                    })),
                    Ok(sip::SipCallAudioRequest {
                        request: Some(AudioRequest::Audio(frame)),
                    }) => respond(AudioResponse::Audio(frame)),
                    Ok(_) => continue,
                    Err(status) => Err(status),
                };
                if sender.send(response).await.is_err() {
                    return;
                }
            }
            let _ = sender
                .send(respond(AudioResponse::Ended(sip::SipCallAudioEnded {
                    reason: sip::SipCallAudioEndReason::ClientClosed as i32,
                    detail: String::new(),
                })))
                .await;
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }
}

/// Start the generated server on an ephemeral loopback port and return it with its address.
///
/// The server task is detached; it ends when the test process does.
async fn start_server() -> (FakeSip, SocketAddr) {
    let service = FakeSip::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    let served = service.clone();
    tokio::spawn(async move {
        Server::builder()
            .add_service(SipServer::new(served))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("the in-process gRPC server must not fail");
    });

    (service, addr)
}

async fn connect(addr: SocketAddr) -> Channel {
    Endpoint::from_shared(format!("http://{addr}"))
        .expect("endpoint")
        .connect_timeout(Duration::from_secs(10))
        .connect()
        .await
        .expect("the in-process gRPC server must accept a connection")
}

#[tokio::test]
async fn a_unary_call_round_trips_through_the_generated_client_and_server() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);

    let status = client
        .sip_start_session(sip::SipStartSessionRequest {
            account_name: "sip-user-1@mydomain.com".to_string(),
            auto_answer_interval: 3,
        })
        .await
        .expect("SipStartSession must succeed")
        .into_inner();

    assert_eq!(status.account_name, "sip-user-1@mydomain.com");
    assert_eq!(status.status_type, StatusType::SessionStarted as i32);
}

/// `ondewo.sip` declares no scalar proto3 `optional` field, so there is no explicit-presence hop
/// to assert here. Its map fields are the equivalent structural risk: a generator that flattened
/// `headers` would lose the SIP headers entirely, and that is invisible in a type signature.
#[tokio::test]
async fn a_map_field_survives_a_real_grpc_hop() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);

    let mut headers = HashMap::new();
    headers.insert("X-Ondewo-Call".to_string(), "outbound".to_string());
    headers.insert("X-Ondewo-Agent".to_string(), "agent-7".to_string());
    headers.insert("X-Ondewo-Empty".to_string(), String::new());

    let echoed = client
        .sip_start_call(sip::SipStartCallRequest {
            callee_id: "sip-user-2@mydomain.com".to_string(),
            headers: headers.clone(),
        })
        .await
        .expect("SipStartCall must succeed")
        .into_inner();

    assert_eq!(echoed.callee_id, "sip-user-2@mydomain.com");
    assert_eq!(
        echoed.headers, headers,
        "every header must come back, including one with an empty value"
    );
}

/// A server-side `Status` has to reach the caller as that same status, not as a transport error.
#[tokio::test]
async fn a_server_error_reaches_the_client_as_its_status() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);

    let error = client
        .sip_start_call(sip::SipStartCallRequest {
            callee_id: UNKNOWN_CALLEE.to_string(),
            headers: HashMap::new(),
        })
        .await
        .expect_err("SipStartCall must report the unknown callee");

    assert_eq!(error.code(), Code::NotFound);
    assert_eq!(
        error.message(),
        "no sip account named does-not-exist@mydomain.com"
    );
}

/// Every RPC the `Sip` proto declares must exist on the generated client and be routable - a
/// method the generator dropped, or wired to the wrong path, fails here with `Unimplemented`.
///
/// The five RPCs that take `google.protobuf.Empty` are called with `()`, which is how prost models
/// that message.
#[tokio::test]
async fn every_declared_service_method_exists_and_is_routable() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);

    client
        .sip_start_session(sip::SipStartSessionRequest::default())
        .await
        .expect("SipStartSession");
    client.sip_end_session(()).await.expect("SipEndSession");
    client
        .sip_start_call(sip::SipStartCallRequest::default())
        .await
        .expect("SipStartCall");
    client
        .sip_end_call(sip::SipEndCallRequest {
            hard_hangup: true,
            ..Default::default()
        })
        .await
        .expect("SipEndCall");
    client
        .sip_transfer_call(sip::SipTransferCallRequest::default())
        .await
        .expect("SipTransferCall");
    client
        .sip_register_account(sip::SipRegisterAccountRequest::default())
        .await
        .expect("SipRegisterAccount");
    client
        .sip_get_sip_status(())
        .await
        .expect("SipGetSipStatus");
    client
        .sip_get_sip_status_history(())
        .await
        .expect("SipGetSipStatusHistory");
    client
        .sip_play_wav_files(sip::SipPlayWavFilesRequest::default())
        .await
        .expect("SipPlayWavFiles");
    client.sip_mute(()).await.expect("SipMute");
    client.sip_un_mute(()).await.expect("SipUnMute");
    client
        .sip_report_answering_machine_detected(
            sip::SipReportAnsweringMachineDetectedRequest::default(),
        )
        .await
        .expect("SipReportAnsweringMachineDetected");
    let mut scoped = Request::new(sip::SipSetCallMediaControlRequest::default());
    scoped
        .metadata_mut()
        .insert(EXPECTED_CALL_ID_METADATA_KEY, "call-1".parse().unwrap());
    client
        .sip_set_call_media_control(scoped)
        .await
        .expect("SipSetCallMediaControl");
    client
        .sip_stream_call_audio(tokio_stream::empty::<sip::SipCallAudioRequest>())
        .await
        .expect("SipStreamCallAudio");
}

/// The answering machine detection result is a nested message with repeated and enum fields; it
/// has to arrive at the server and come back in `SipStatus.amd_result` intact.
#[tokio::test]
async fn an_answering_machine_detection_result_round_trips() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);
    let amd_result = sip::AnsweringMachineDetectionResult {
        verdict: sip::answering_machine_detection_result::Verdict::Machine as i32,
        confidence: 0.75,
        decision_ms: 1_800,
        rule_id: "rule-1".to_string(),
        matched_cue_ids: vec!["beep".to_string(), "greeting".to_string()],
        call_id: "call-1".to_string(),
        ..Default::default()
    };

    let status = client
        .sip_report_answering_machine_detected(sip::SipReportAnsweringMachineDetectedRequest {
            amd_result: Some(amd_result.clone()),
        })
        .await
        .expect("SipReportAnsweringMachineDetected must succeed")
        .into_inner();

    assert_eq!(
        status.status_type,
        StatusType::OutgoingCallAnsweringMachineDetected as i32
    );
    assert_eq!(status.amd_result, Some(amd_result));
    assert_eq!(status.call_id, "call-1");
}

/// `SipSetCallMediaControl` is call-scoped through gRPC metadata, not through a request field, so
/// the metadatum a caller attaches has to reach the server.
#[tokio::test]
async fn the_call_scope_metadatum_reaches_the_server() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);
    let control = sip::SipSetCallMediaControlRequest {
        bot_voice: sip::MediaControlSetting::Off as i32,
        bot_listening: sip::MediaControlSetting::On as i32,
        owner: sip::MediaControlOwner::Operator as i32,
        participants_present: false,
    };

    let unscoped = client
        .sip_set_call_media_control(control)
        .await
        .expect_err("the fake server refuses a request without the call scope");
    assert_eq!(unscoped.code(), Code::FailedPrecondition);

    let mut scoped = Request::new(control);
    scoped
        .metadata_mut()
        .insert(EXPECTED_CALL_ID_METADATA_KEY, "call-7".parse().unwrap());
    let status = client
        .sip_set_call_media_control(scoped)
        .await
        .expect("SipSetCallMediaControl must succeed with the call scope")
        .into_inner();

    assert_eq!(status.call_id, "call-7");
    assert!(status.bot_muted);
    assert!(!status.listening_paused);
}

/// The bidirectional `SipStreamCallAudio` has to carry both directions: the oneof of every request
/// reaches the server, and every response - including the audio bytes - comes back in order.
#[tokio::test]
async fn the_call_audio_stream_carries_both_directions() {
    let (_service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);
    let frame = sip::SipCallAudioFrame {
        pcm_s16le: vec![0x01, 0x00, 0xFF, 0x7F],
        sequence: 1,
    };
    let requests = vec![
        sip::SipCallAudioRequest {
            request: Some(AudioRequest::Config(sip::SipCallAudioConfig {
                mode: sip::SipCallAudioMode::Listen as i32,
                sample_rate_hz: 16_000,
                frame_ms: 20,
                stream_id: "stream-1".to_string(),
                ..Default::default()
            })),
        },
        sip::SipCallAudioRequest {
            request: Some(AudioRequest::Audio(frame.clone())),
        },
        sip::SipCallAudioRequest {
            request: Some(AudioRequest::AgentMuted(true)),
        },
    ];

    let responses: Vec<AudioResponse> = client
        .sip_stream_call_audio(tokio_stream::iter(requests))
        .await
        .expect("SipStreamCallAudio must open")
        .into_inner()
        .map(|message| {
            message
                .expect("every response must be Ok")
                .response
                .unwrap()
        })
        .collect()
        .await;

    assert_eq!(
        responses,
        vec![
            AudioResponse::Started(sip::SipCallAudioStarted {
                stream_id: "stream-1".to_string(),
                sample_rate_hz: 16_000,
                frame_ms: 20,
                mode: sip::SipCallAudioMode::Listen as i32,
            }),
            AudioResponse::Audio(frame),
            AudioResponse::Ended(sip::SipCallAudioEnded {
                reason: sip::SipCallAudioEndReason::ClientClosed as i32,
                detail: String::new(),
            }),
        ]
    );
}

/// The hand-written [`BearerTokenInterceptor`] has to put its metadata on the wire, where the
/// server can actually read it - asserting on the `Request` it returns would not prove that.
#[tokio::test]
async fn the_bearer_interceptor_reaches_the_server() {
    let (service, addr) = start_server().await;
    let interceptor = BearerTokenInterceptor::new("access-token-abc")
        .expect("a plain ASCII token is valid")
        .with_cai_token("cai-token-xyz")
        .expect("a plain ASCII cai token is valid");
    let mut client = SipClient::with_interceptor(connect(addr).await, interceptor);

    client
        .sip_get_sip_status(())
        .await
        .expect("SipGetSipStatus");

    assert_eq!(
        service.seen(),
        SeenMetadata {
            authorization: Some("Bearer access-token-abc".to_string()),
            cai_token: Some("cai-token-xyz".to_string()),
        }
    );
}

/// Without the interceptor the client must send no credentials at all - the unauthenticated path
/// (plaintext server, or an ingress that injects the bearer token) has to stay usable.
#[tokio::test]
async fn a_client_without_an_interceptor_sends_no_credentials() {
    let (service, addr) = start_server().await;
    let mut client = SipClient::new(connect(addr).await);

    client
        .sip_get_sip_status(())
        .await
        .expect("SipGetSipStatus");

    assert_eq!(service.seen(), SeenMetadata::default());
}

/// A client built against an address nothing listens on must surface a transport error rather
/// than panic or hang - `connect_lazy` defers the connect to the first call.
#[tokio::test]
async fn a_call_to_an_unreachable_target_fails_as_a_status() {
    let channel = Endpoint::from_static("http://127.0.0.1:1")
        .connect_timeout(Duration::from_secs(2))
        .connect_lazy();
    let mut client = SipClient::new(channel);

    let error = client
        .sip_get_sip_status(())
        .await
        .expect_err("nothing listens on port 1");

    assert_eq!(error.code(), Code::Unavailable);
}
