// src/main.rs
use anyhow::{anyhow, Context, Result};
use cynic::QueryBuilder;
use fuel_core_client::client::{
    pagination::{PageDirection, PaginatedResult, PaginationRequest},
    schema::{
        block::Header,
        schema::{self},
        tx::TransactionStatus,
        BlockId, ConnectionArgs, HexString, PageInfo, TransactionId,
    },
    FuelClient,
};
use fuel_merkle::binary::root_calculator::MerkleRootCalculator;
use fuel_tx::{field::ReceiptsRoot, Receipt, Transaction};
use fuel_types::{
    canonical::{Deserialize, Serialize},
    Bytes32,
};
use std::sync::Arc;

// Custom query fragments similar to full_block_query.rs
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./src/schema.sdl",
    graphql_type = "Query",
    variables = "ConnectionArgs"
)]
pub struct FullBlocksQuery {
    #[arguments(after: $after, before: $before, first: $first, last: $last)]
    pub blocks: FullBlockConnection,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(graphql_type = "Transaction", schema_path = "./src/schema.sdl")]
pub struct OpaqueTransactionWithStatusAndId {
    pub id: TransactionId,
    pub raw_payload: HexString,
    pub status: Option<TransactionStatus>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./src/schema.sdl", graphql_type = "BlockConnection")]
pub struct FullBlockConnection {
    pub edges: Vec<FullBlockEdge>,
    pub page_info: PageInfo,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./src/schema.sdl", graphql_type = "BlockEdge")]
pub struct FullBlockEdge {
    pub cursor: String,
    pub node: FullBlock,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "./src/schema.sdl", graphql_type = "Block")]
pub struct FullBlock {
    pub id: BlockId,
    pub header: Header,
    pub transactions: Vec<OpaqueTransactionWithStatusAndId>,
}

impl From<FullBlockConnection> for PaginatedResult<FullBlock, String> {
    fn from(conn: FullBlockConnection) -> Self {
        PaginatedResult {
            cursor: conn.page_info.end_cursor,
            has_next_page: conn.page_info.has_next_page,
            has_previous_page: conn.page_info.has_previous_page,
            results: conn.edges.into_iter().map(|e| e.node).collect(),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();

    // Create fuel client
    let fuel_client = Arc::new(FuelClient::new("https://testnet.fuel.network/v1/graphql")?);

    // Example block height to validate
    let block_height = 3674822;

    // Query for the block using our custom query
    let blocks = fuel_client
        .query(FullBlocksQuery::build(
            PaginationRequest {
                cursor: Some((block_height - 1).to_string()),
                results: 1,
                direction: PageDirection::Forward,
            }
            .into(),
        ))
        .await
        .context("failed to query block")?;

    let block = blocks
        .blocks
        .edges
        .first()
        .ok_or_else(|| anyhow!("no block found"))?
        .node
        .clone();

    println!("Validating block height: {}", block_height);

    // Validate transaction root
    let tx_root: Bytes32 = block.header.transactions_root.clone().into();
    let mut calculated_tx_root = MerkleRootCalculator::new();

    for tx in &block.transactions {
        let tx_id = tx.id.to_string();

        let receipts = match &tx.status {
            Some(TransactionStatus::SuccessStatus(status)) => &status.receipts,
            Some(TransactionStatus::FailureStatus(status)) => {
                println!(
                    "Found failed transaction: {} with reason: {}",
                    tx_id, status.reason
                );
                &status.receipts
            }
            _ => continue,
        };

        // Parse transaction
        let tx_body = Transaction::from_bytes(tx.raw_payload.0 .0.as_slice())
            .map_err(|e| anyhow!("{e}"))
            .context("failed to parse transaction")?;

        // Add to merkle tree
        calculated_tx_root.push(&tx_body.to_bytes());

        // Validate receipt root for Script transactions
        if let Transaction::Script(tx_body) = tx_body {
            let receipt_root = *tx_body.receipts_root();
            let mut calculated_receipt_root = MerkleRootCalculator::new();

            for receipt in receipts {
                let receipt: Receipt = receipt.clone().try_into()?;
                calculated_receipt_root.push(receipt.to_bytes().as_slice());
            }

            let calculated_receipt_root: Bytes32 = calculated_receipt_root.root().into();

            if receipt_root != calculated_receipt_root {
                println!(
                    "Receipt root mismatch for failed transaction {} [in block #{:?}]: expected {}, got {}",
                    tx_id, block.id, receipt_root, calculated_receipt_root
                );
                return Err(anyhow!(
                    "Receipt root mismatch for failed transaction {} [in block #{:?}]: expected {}, got {}",
                    tx_id, block.id, receipt_root, calculated_receipt_root
                ));
            }
        }
    }

    // Validate final transaction root
    let calculated_tx_root = calculated_tx_root.root().into();
    if tx_root != calculated_tx_root {
        println!(
            "Transaction root mismatch (with failed transactions): expected {}, got {}",
            tx_root, calculated_tx_root
        );
        return Err(anyhow!(
            "Transaction root mismatch: expected {}, got {}",
            tx_root,
            calculated_tx_root
        ));
    }

    println!("Block validation completed successfully!");
    Ok(())
}
