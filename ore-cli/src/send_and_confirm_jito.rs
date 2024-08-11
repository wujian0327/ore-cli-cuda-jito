use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use crate::utils::get_proof_with_authority;
use crate::Miner;
use jito_protos::searcher::searcher_service_client::SearcherServiceClient;
use jito_protos::{
    auth::{auth_service_client::AuthServiceClient, Role},
    bundle::Bundle,
    convert::proto_packet_from_versioned_tx,
    searcher::{SendBundleRequest, SendBundleResponse},
};
use searcher_client::token_authenticator::ClientInterceptor;
use solana_client::rpc_client::RpcClient;
use solana_program::instruction::Instruction;
use solana_program::pubkey;
use solana_sdk::message::{v0, VersionedMessage};
use solana_sdk::system_instruction::transfer;
use solana_sdk::transaction::VersionedTransaction;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::{read_keypair_file, Keypair, Signer},
};
use thiserror::Error;
use tonic::{
    codegen::InterceptedService,
    transport,
    transport::{Channel, Endpoint},
    Response, Status,
};

impl Miner {
    pub async fn send_and_confirm_d_jito(&self, ixs: &[Instruction]) {
        let jito_auth = self.joti_auth.clone();
        let feepayer = read_keypair_file(jito_auth.clone()).unwrap();
        let signer = self.signer();
        let auth: Arc<Keypair> = Arc::new(read_keypair_file(jito_auth).unwrap());
        let rpc_client = RpcClient::new_with_commitment(
            self.rpc_client.url().clone(),
            CommitmentConfig::confirmed(),
        );
        let blockhash = rpc_client.get_latest_blockhash().expect("get blockhash");
        let mut versioned_txs = Vec::new();

        let mut vec_signers = vec![&signer];
        vec_signers.insert(0, &feepayer);

        let mut vec_ixs = Vec::from(ixs);

        let lamports = self.joti_fee;

        let jito_tip_ix = transfer(
            &feepayer.pubkey(),
            &pubkey!("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5"),
            lamports,
        );
        vec_ixs.push(jito_tip_ix);

        let versioned_tx = VersionedTransaction::try_new(
            VersionedMessage::V0(
                v0::Message::try_compile(&feepayer.pubkey(), &vec_ixs, &[], blockhash).unwrap(),
            ),
            &vec_signers,
        )
        .unwrap();

        versioned_txs.push(versioned_tx);

        let mut searcher_client =
            get_searcher_client("https://amsterdam.mainnet.block-engine.jito.wtf", &auth)
                .await
                .unwrap();

        // Send the bundle of versioned transactions
        let proof: ore_api::state::Proof =
            get_proof_with_authority(&self.rpc_client, signer.pubkey()).await;
        let challange_old = proof.challenge;
        let send_response = send_bundle_no_wait(&versioned_txs, &mut searcher_client)
            .await
            .unwrap();
        println!("Bundle sent with uuid {}", send_response.into_inner().uuid);

        for i in 0..10 {
            sleep(Duration::from_secs(3));
            let proof = get_proof_with_authority(&self.rpc_client, signer.pubkey()).await;
            let challange_new = proof.challenge;
            if challange_old != challange_new {
                println!("transaction landed: new challange:{:?}", challange_new);
                break;
            } else {
                println!("wait for transaction landed: {i} times");
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum BlockEngineConnectionError {
    #[error("transport error {0}")]
    TransportError(#[from] transport::Error),
    #[error("client error {0}")]
    ClientError(#[from] Status),
}
pub type BlockEngineConnectionResult<T> = Result<T, BlockEngineConnectionError>;

async fn get_searcher_client(
    block_engine_url: &str,
    auth_keypair: &Arc<Keypair>,
) -> BlockEngineConnectionResult<
    SearcherServiceClient<InterceptedService<Channel, ClientInterceptor>>,
> {
    let auth_channel = create_grpc_channel(block_engine_url).await?;
    let client_interceptor = ClientInterceptor::new(
        AuthServiceClient::new(auth_channel),
        auth_keypair,
        Role::Searcher,
    )
    .await
    .unwrap();

    let searcher_channel = create_grpc_channel(block_engine_url).await?;
    let searcher_client =
        SearcherServiceClient::with_interceptor(searcher_channel, client_interceptor);
    Ok(searcher_client)
}

async fn create_grpc_channel(url: &str) -> BlockEngineConnectionResult<Channel> {
    let mut endpoint = Endpoint::from_shared(url.to_string()).expect("invalid url");
    if url.starts_with("https") {
        endpoint = endpoint.tls_config(tonic::transport::ClientTlsConfig::new())?;
    }
    Ok(endpoint.connect().await?)
}

pub async fn send_bundle_no_wait(
    transactions: &[VersionedTransaction],
    searcher_client: &mut SearcherServiceClient<InterceptedService<Channel, ClientInterceptor>>,
) -> Result<Response<SendBundleResponse>, Status> {
    // convert them to packets + send over
    let packets: Vec<_> = transactions
        .iter()
        .map(proto_packet_from_versioned_tx)
        .collect();

    searcher_client
        .send_bundle(SendBundleRequest {
            bundle: Some(Bundle {
                header: None,
                packets,
            }),
        })
        .await
}
