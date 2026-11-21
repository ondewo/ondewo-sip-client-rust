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

//! Call an ONDEWO SIP server with an auto-refreshed Keycloak token.
//!
//! With `ONDEWO_SIP_OFFLINE_TOKEN` set, the client starts up from that offline refresh token and
//! never sends a password; otherwise it logs in with `ONDEWO_SIP_PASSWORD`.
//!
//! ```sh
//! ONDEWO_SIP_HOST=sip.example.com ONDEWO_SIP_PORT=443 \
//! ONDEWO_KEYCLOAK_URL=https://keycloak.example.com/auth \
//! ONDEWO_KEYCLOAK_REALM=ondewo-ccai-platform \
//! ONDEWO_KEYCLOAK_CLIENT_ID=ondewo-sip-cai-sdk-public \
//! ONDEWO_SIP_USERNAME=technical-user@example.com \
//! ONDEWO_SIP_OFFLINE_TOKEN=<offline refresh token> \
//!   cargo run --example keycloak_client
//! ```

use std::env;
use std::error::Error;

use ondewo_sip_client::api::ondewo::sip;
use ondewo_sip_client::api::ondewo::sip::sip_client::SipClient;
use ondewo_sip_client::channel::ClientConfig;
use ondewo_sip_client::keycloak::{KeycloakConfig, KeycloakInterceptor};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = KeycloakConfig::new(
        env::var("ONDEWO_KEYCLOAK_URL")?,
        env::var("ONDEWO_KEYCLOAK_REALM")?,
        env::var("ONDEWO_KEYCLOAK_CLIENT_ID")?,
        env::var("ONDEWO_SIP_USERNAME").unwrap_or_default(),
        env::var("ONDEWO_SIP_PASSWORD").unwrap_or_default(),
    )
    .with_refresh_token(env::var("ONDEWO_SIP_OFFLINE_TOKEN").unwrap_or_default());
    // Validates the config, then acquires the first token; the provider keeps it fresh.
    let interceptor = KeycloakInterceptor::new(&config)?;

    let channel = ClientConfig::new(
        env::var("ONDEWO_SIP_HOST")?,
        env::var("ONDEWO_SIP_PORT")?.parse()?,
    )
    .connect()
    .await?;
    let mut client = SipClient::with_interceptor(channel, interceptor);

    // `SipGetSipStatus` takes google.protobuf.Empty, which prost models as `()`.
    let status = client.sip_get_sip_status(()).await?.into_inner();

    let status_type = sip::sip_status::StatusType::try_from(status.status_type)
        .map(|status_type| status_type.as_str_name().to_string())
        .unwrap_or_else(|_| format!("<unknown status {}>", status.status_type));
    println!("{} is {}", status.account_name, status_type);
    Ok(())
}
