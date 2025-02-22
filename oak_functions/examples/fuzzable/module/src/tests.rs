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

use crate::proto::{
    instruction::InstructionVariant, Instruction, Instructions, Panic, WriteResponse,
};
use oak_functions_abi::proto::StatusCode;
use oak_functions_loader::{
    grpc::create_and_start_grpc_server,
    logger::Logger,
    lookup::{LookupData, LookupDataAuth},
    server::Policy,
};
use prost::Message;
use std::{
    net::{Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use test_utils::make_request;

#[tokio::test]
async fn test_server() {
    let server_port = test_utils::free_port();
    let address = SocketAddr::from((Ipv6Addr::UNSPECIFIED, server_port));

    let mut manifest_path = std::env::current_dir().unwrap();
    manifest_path.push("Cargo.toml");

    let wasm_module_bytes =
        test_utils::compile_rust_wasm(manifest_path.to_str().expect("Invalid target dir"), false)
            .expect("Couldn't read Wasm module");

    let logger = Logger::for_test();

    let lookup_data = Arc::new(LookupData::new_empty(
        "",
        LookupDataAuth::default(),
        logger.clone(),
    ));

    let policy = Policy {
        constant_response_size_bytes: 100,
        constant_processing_time: Duration::from_millis(200),
    };
    let tee_certificate = vec![];

    let server_background = test_utils::background(|term| async move {
        create_and_start_grpc_server(
            &address,
            tee_certificate,
            &wasm_module_bytes,
            lookup_data,
            None,
            policy,
            term,
            logger,
            None,
        )
        .await
    });

    {
        // Send a request with an empty instruction list.
        let request = Instructions {
            instructions: vec![],
        };
        let mut request_bytes = vec![];
        request
            .encode(&mut request_bytes)
            .expect("Couldn't encode empty instruction list");
        let response = make_request(server_port, &request_bytes).await.response;
        assert_eq!(StatusCode::Success as i32, response.status,);
        assert_eq!(b"Done fuzzing!", response.body().unwrap());
    }

    {
        // Send a request to simulate a panic.
        let request = Instructions {
            instructions: vec![Instruction {
                instruction_variant: Some(InstructionVariant::Panic(Panic {})),
            }],
        };
        let mut request_bytes = vec![];
        request
            .encode(&mut request_bytes)
            .expect("Couldn't encode a single panic instruction");
        let response = make_request(server_port, &request_bytes).await.response;
        assert_eq!(StatusCode::Success as i32, response.status);

        // Expect an empty response.
        assert_eq!(0, response.body().unwrap().len());
    }

    {
        // Send a request to simulate a write_response followed by a panic.
        let request = Instructions {
            instructions: vec![
                Instruction {
                    instruction_variant: Some(InstructionVariant::WriteResponse(WriteResponse {
                        response: br"Random response!".to_vec(),
                    })),
                },
                Instruction {
                    instruction_variant: Some(InstructionVariant::Panic(Panic {})),
                },
            ],
        };
        let mut request_bytes = vec![];
        request
            .encode(&mut request_bytes)
            .expect("Couldn't encode instruction list");
        let response = make_request(server_port, &request_bytes).await.response;
        assert_eq!(StatusCode::Success as i32, response.status);

        // Expect non-empty response.
        assert_eq!(b"Random response!", response.body().unwrap());
    }

    let res = server_background.terminate_and_join().await;
    assert!(res.is_ok());
}
