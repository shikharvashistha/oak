//
// Copyright 2021 The Project Oak Authors
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
//

//! gRPC server for Oak Functions.

use crate::{
    attestation::AttestationServer,
    logger::Logger,
    lookup::LookupData,
    metrics::PrivateMetricsAggregator,
    proto::remote_attestation_server::RemoteAttestationServer,
    server::{apply_policy, Policy, WasmHandler},
    tf::TensorFlowModel,
};
use anyhow::Context;
use log::Level;
use oak_functions_abi::proto::Request;
use prost::Message;
use std::{
    convert::TryInto,
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

async fn handle_request(
    wasm_handler: WasmHandler,
    policy: Policy,
    request: Request,
) -> anyhow::Result<Vec<u8>> {
    let function = async move || wasm_handler.clone().handle_invoke(request).await;
    let policy = policy.try_into().context("invalid policy")?;
    let response = apply_policy(policy, function)
        .await
        .context("internal error")?;
    let mut bytes = vec![];
    response
        .encode(&mut bytes)
        .context("couldn't encode response")?;
    Ok(bytes)
}

/// Starts a gRPC server on the given address, serving the `main` function of the given Wasm module.
#[allow(clippy::too_many_arguments)]
pub async fn create_and_start_grpc_server<F: Future<Output = ()>>(
    address: &SocketAddr,
    tee_certificate: Vec<u8>,
    wasm_module_bytes: &[u8],
    lookup_data: Arc<LookupData>,
    tf_model: Option<TensorFlowModel>,
    policy: Policy,
    terminate: F,
    logger: Logger,
    aggregator: Option<Arc<Mutex<PrivateMetricsAggregator>>>,
) -> anyhow::Result<()> {
    let wasm_handler = WasmHandler::create(
        wasm_module_bytes,
        lookup_data,
        Arc::new(tf_model),
        logger.clone(),
        aggregator,
    )?;

    logger.log_public(
        Level::Info,
        &format!(
            "{:?}: Starting gRPC server on {:?}",
            std::thread::current().id(),
            &address
        ),
    );

    let request_handler = async move |request| handle_request(wasm_handler, policy, request).await;

    // A `Service` is needed for every connection. Here we create a service using the
    // `wasm_handler`.
    tonic::transport::Server::builder()
        .add_service(RemoteAttestationServer::new(
            AttestationServer::create(tee_certificate, request_handler)
                .context("Couldn't create remote attestation server")?,
        ))
        .serve_with_shutdown(*address, terminate)
        .await
        .context("Couldn't start server")?;

    Ok(())
}
