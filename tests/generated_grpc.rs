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
//! `ondewo.sip.Sip` is entirely unary - it declares no streaming RPC - so every case below is a
//! plain request/response hop.
//!
//! No network beyond `127.0.0.1` and no ONDEWO server is involved.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ondewo_sip_client::api::ondewo::sip;
use ondewo_sip_client::api::ondewo::sip::sip_client::SipClient;
use ondewo_sip_client::api::ondewo::sip::sip_server::{Sip, SipServer};
use ondewo_sip_client::auth::{
    BearerTokenInterceptor, AUTHORIZATION_METADATA_KEY, CAI_TOKEN_METADATA_KEY,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Code, Request, Response, Status};

use sip::sip_status::StatusType;

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
        .sip_end_call(sip::SipEndCallRequest { hard_hangup: true })
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
